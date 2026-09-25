//! `gitlab_group` provider: `GET /groups/{path}/members/all/{user id}`
//! (includes inherited membership) with the user's existing credentials
//!. Tokens are never stored and never logged.

use std::sync::Mutex;
use std::time::Duration;

use loadout_git::Git;
use secrecy::{ExposeSecret, SecretString};

use crate::{GitlabGroups, ProviderError};

/// Default API endpoint; `LOADOUT_GITLAB_API` overrides it (self-managed
/// GitLab, tests).
pub const DEFAULT_API: &str = "https://gitlab.com/api/v4";

const NAME: &str = "gitlab_group";

pub struct GitlabClient {
    api: String,
    token: Result<SecretString, String>,
    user_id: Mutex<Option<Result<u64, ProviderError>>>,
    agent: ureq::Agent,
}

impl GitlabClient {
    /// Finds the API base and a token: `GITLAB_TOKEN`, `GL_TOKEN`, then the
    /// Git credential helper for the GitLab host.
    pub fn discover(env: &dyn Fn(&str) -> Option<String>, git: &Git) -> Self {
        let api = env("LOADOUT_GITLAB_API")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_API.to_owned())
            .trim_end_matches('/')
            .to_owned();
        let host = api
            .split_once("://")
            .map_or(api.as_str(), |(_, r)| r)
            .split('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        let token = ["GITLAB_TOKEN", "GL_TOKEN"]
            .into_iter()
            .filter_map(env)
            .find(|t| !t.trim().is_empty())
            .map(|t| SecretString::from(t.trim().to_owned()))
            .or_else(|| token_from_git(git, &host))
            .ok_or_else(|| {
                format!("no GitLab token (set GITLAB_TOKEN or configure a Git credential helper for {host})")
            });
        GitlabClient::new(api, token)
    }

    pub fn new(api: String, token: Result<SecretString, String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .user_agent(concat!("loadout/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        GitlabClient {
            api,
            token,
            user_id: Mutex::new(None),
            agent,
        }
    }

    fn get(&self, path: &str) -> Result<(u16, String), ProviderError> {
        let err = |m: String| ProviderError::new(NAME, m);
        let token = self.token.as_ref().map_err(|m| err(m.clone()))?;
        let url = format!("{}{path}", self.api);
        let mut resp = self
            .agent
            .get(&url)
            .header("Authorization", format!("Bearer {}", token.expose_secret()))
            .call()
            .map_err(|e| err(format!("GET {url}: {e}")))?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| err(format!("GET {url}: {e}")))?;
        Ok((status, body))
    }

    fn user_id(&self) -> Result<u64, ProviderError> {
        let mut cached = self.user_id.lock().expect("user cache poisoned");
        if let Some(r) = &*cached {
            return r.clone();
        }
        let r = match self.get("/user") {
            Ok((200, body)) => serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["id"].as_u64())
                .ok_or_else(|| ProviderError::new(NAME, "unexpected /user response")),
            Ok((status, _)) => Err(ProviderError::new(
                NAME,
                format!("GET /user returned HTTP {status} (is the token valid?)"),
            )),
            Err(e) => Err(e),
        };
        *cached = Some(r.clone());
        r
    }
}

/// Percent-encodes a group path (`acme/payments` → `acme%2Fpayments`).
fn encode_path(p: &str) -> String {
    p.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

impl GitlabGroups for GitlabClient {
    fn is_member(&self, group: &str) -> Result<bool, ProviderError> {
        let id = self.user_id()?;
        let path = format!("/groups/{}/members/all/{id}", encode_path(group));
        match self.get(&path)? {
            (200, body) => {
                let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
                    ProviderError::new(NAME, format!("bad response for {path}: {e}"))
                })?;
                // Blocked or awaiting members are not members.
                Ok(v["state"].as_str().is_none_or(|s| s == "active"))
            }
            (404, _) => Ok(false),
            (status, _) => Err(ProviderError::new(
                NAME,
                format!("GET {path} returned HTTP {status} (the token may lack read_api)"),
            )),
        }
    }
}

/// A [`GitlabClient`] created on first use.
pub struct LazyGitlab<'a> {
    env: &'a dyn Fn(&str) -> Option<String>,
    git: &'a Git,
    client: std::sync::OnceLock<GitlabClient>,
}

impl<'a> LazyGitlab<'a> {
    pub fn new(env: &'a dyn Fn(&str) -> Option<String>, git: &'a Git) -> Self {
        LazyGitlab {
            env,
            git,
            client: std::sync::OnceLock::new(),
        }
    }
}

impl GitlabGroups for LazyGitlab<'_> {
    fn is_member(&self, group: &str) -> Result<bool, ProviderError> {
        self.client
            .get_or_init(|| GitlabClient::discover(self.env, self.git))
            .is_member(group)
    }
}

fn token_from_git(git: &Git, host: &str) -> Option<SecretString> {
    let out = git
        .run_with_stdin(
            None,
            ["credential", "fill"],
            format!("protocol=https\nhost={host}\n\n").as_bytes(),
        )
        .ok()?;
    out.lines()
        .find_map(|l| l.strip_prefix("password="))
        .filter(|p| !p.is_empty())
        .map(|p| SecretString::from(p.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_group_paths() {
        assert_eq!(encode_path("acme/payments eng"), "acme%2Fpayments%20eng");
    }

    #[test]
    fn missing_token_is_a_provider_error() {
        let c = GitlabClient::new("http://127.0.0.1:9".into(), Err("no token".into()));
        assert_eq!(c.is_member("acme").unwrap_err().message, "no token");
    }
}

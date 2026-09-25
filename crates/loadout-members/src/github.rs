//! `github_team` provider: `GET /orgs/{org}/teams/{team}/memberships/{user}`
//! with the user's existing credentials. Tokens are never stored
//! and never logged.

use std::sync::Mutex;
use std::time::Duration;

use loadout_git::Git;
use secrecy::{ExposeSecret, SecretString};

use crate::{GithubTeams, ProviderError};

/// Default API endpoint; `LOADOUT_GITHUB_API` overrides it (GitHub
/// Enterprise Server, tests).
pub const DEFAULT_API: &str = "https://api.github.com";

pub struct GithubClient {
    api: String,
    token: Result<SecretString, String>,
    login: Mutex<Option<Result<String, ProviderError>>>,
    agent: ureq::Agent,
}

impl GithubClient {
    /// Finds the API base and a token: `GH_TOKEN`, `GITHUB_TOKEN`,
    /// `gh auth token`, then the Git credential helper.
    pub fn discover(env: &dyn Fn(&str) -> Option<String>, git: &Git) -> Self {
        let api = env("LOADOUT_GITHUB_API")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_API.to_owned())
            .trim_end_matches('/')
            .to_owned();
        let host = web_host(&api);
        let token = token_from_env(env)
            .or_else(|| token_from_gh(&host))
            .or_else(|| token_from_git(git, &host))
            .ok_or_else(|| {
                format!(
                    "no GitHub token (set GH_TOKEN, run `gh auth login`, or configure a Git \
                     credential helper for {host})"
                )
            });
        GithubClient::new(api, token)
    }

    pub fn new(api: String, token: Result<SecretString, String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .user_agent(concat!("loadout/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        GithubClient {
            api,
            token,
            login: Mutex::new(None),
            agent,
        }
    }

    fn get(&self, path: &str) -> Result<(u16, String), ProviderError> {
        let err = |m: String| ProviderError::new("github_team", m);
        let token = self.token.as_ref().map_err(|m| err(m.clone()))?;
        let url = format!("{}{path}", self.api);
        let mut resp = self
            .agent
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
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

    fn login(&self) -> Result<String, ProviderError> {
        let mut cached = self.login.lock().expect("login cache poisoned");
        if let Some(r) = &*cached {
            return r.clone();
        }
        let r = match self.get("/user") {
            Ok((200, body)) => serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["login"].as_str().map(str::to_owned))
                .ok_or_else(|| ProviderError::new("github_team", "unexpected /user response")),
            Ok((status, _)) => Err(ProviderError::new(
                "github_team",
                format!("GET /user returned HTTP {status} (is the token valid?)"),
            )),
            Err(e) => Err(e),
        };
        *cached = Some(r.clone());
        r
    }
}

impl GithubTeams for GithubClient {
    fn is_member(&self, org: &str, team: &str) -> Result<bool, ProviderError> {
        let login = self.login()?;
        let path = format!("/orgs/{org}/teams/{team}/memberships/{login}");
        match self.get(&path)? {
            (200, body) => {
                let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
                    ProviderError::new("github_team", format!("bad response for {path}: {e}"))
                })?;
                Ok(v["state"] == "active")
            }
            (404, _) => Ok(false),
            (status, _) => Err(ProviderError::new(
                "github_team",
                format!(
                    "GET {path} returned HTTP {status} (the token may lack the read:org scope)"
                ),
            )),
        }
    }
}

/// A [`GithubClient`] created on first use, so company configs without
/// `github_team` rules never look for a token.
pub struct LazyGithub<'a> {
    env: &'a dyn Fn(&str) -> Option<String>,
    git: &'a Git,
    client: std::sync::OnceLock<GithubClient>,
}

impl<'a> LazyGithub<'a> {
    pub fn new(env: &'a dyn Fn(&str) -> Option<String>, git: &'a Git) -> Self {
        LazyGithub {
            env,
            git,
            client: std::sync::OnceLock::new(),
        }
    }
}

impl GithubTeams for LazyGithub<'_> {
    fn is_member(&self, org: &str, team: &str) -> Result<bool, ProviderError> {
        self.client
            .get_or_init(|| GithubClient::discover(self.env, self.git))
            .is_member(org, team)
    }
}

/// `https://api.github.com` → `github.com`; `https://ghe.example.com/api/v3`
/// → `ghe.example.com`.
fn web_host(api: &str) -> String {
    let rest = api.split_once("://").map_or(api, |(_, r)| r);
    let host = rest.split('/').next().unwrap_or(rest);
    host.strip_prefix("api.").unwrap_or(host).to_owned()
}

/// A GitHub token for `host` from the user's existing credentials:
/// `GH_TOKEN`, `GITHUB_TOKEN`, `gh auth token`, then the Git credential
/// helper. Never stored.
pub fn discover_token(
    env: &dyn Fn(&str) -> Option<String>,
    git: &Git,
    host: &str,
) -> Option<SecretString> {
    token_from_env(env)
        .or_else(|| token_from_gh(host))
        .or_else(|| token_from_git(git, host))
}

/// The web host for an API base (`https://api.github.com` → `github.com`).
pub fn api_host(api: &str) -> String {
    web_host(api)
}

fn token_from_env(env: &dyn Fn(&str) -> Option<String>) -> Option<SecretString> {
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .into_iter()
        .filter_map(env)
        .find(|t| !t.trim().is_empty())
        .map(|t| SecretString::from(t.trim().to_owned()))
}

fn token_from_gh(host: &str) -> Option<SecretString> {
    let out = std::process::Command::new("gh")
        .args(["auth", "token", "--hostname", host])
        .output()
        .ok()?;
    let token = String::from_utf8(out.stdout).ok()?;
    (out.status.success() && !token.trim().is_empty())
        .then(|| SecretString::from(token.trim().to_owned()))
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
    fn web_hosts() {
        assert_eq!(web_host("https://api.github.com"), "github.com");
        assert_eq!(
            web_host("https://ghe.example.com/api/v3"),
            "ghe.example.com"
        );
        assert_eq!(web_host("http://127.0.0.1:9"), "127.0.0.1:9");
    }

    #[test]
    fn env_token_order() {
        let env = |k: &str| match k {
            "GH_TOKEN" => Some("  ".to_owned()),
            "GITHUB_TOKEN" => Some("t2".to_owned()),
            _ => None,
        };
        assert_eq!(token_from_env(&env).unwrap().expose_secret(), "t2");
        assert!(token_from_env(&|_| None).is_none());
    }

    #[test]
    fn missing_token_is_a_provider_error() {
        let c = GithubClient::new("http://127.0.0.1:9".into(), Err("no token".into()));
        let e = c.is_member("acme", "eng").unwrap_err();
        assert_eq!(e.message, "no token");
    }
}

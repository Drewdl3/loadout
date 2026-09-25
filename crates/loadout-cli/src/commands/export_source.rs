//! `lo export-source <source> [--branch] [--message] [--prepare]`:
//! commit the edits in a source's working clone to a new
//! `loadout/<user>/<desc>` branch, push it, and open a pull request. Never
//! pushes the default branch and never merges.

use std::fmt::Write as _;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;
use loadout_git::Git;
use schemars::JsonSchema;
use secrecy::ExposeSecret;
use serde::Serialize;

use crate::ctx::{Ctx, Report};
use crate::exit;
use crate::work;

#[derive(Debug, Args)]
pub struct ExportSourceArgs {
    /// The source (its `LOADOUT.md` name).
    pub source: String,
    /// Branch to create (must start with `loadout/`; default
    /// `loadout/<user>/<message slug>`).
    #[arg(long)]
    pub branch: Option<String>,
    /// Commit message (also the pull request title).
    #[arg(long, short = 'm', default_value = "Update agent configuration")]
    pub message: String,
    /// Only make sure the working clone exists and print its path (edit it,
    /// then run export-source again).
    #[arg(long)]
    pub prepare: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PullRequest {
    pub url: String,
    pub number: u64,
}

/// `--json` output of `lo export-source`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ExportSourceReport {
    pub source: String,
    /// The working clone.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub pushed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequest>,
    pub warnings: Vec<String>,
}

impl Report for ExportSourceReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        match (&self.branch, &self.pull_request) {
            (None, _) => writeln!(
                out,
                "Working clone of {}: {}\nEdit it, then run `lo export-source {} -m \"…\"`.",
                self.source, self.path, self.source
            ),
            (Some(b), Some(pr)) => {
                writeln!(out, "Pushed {b}; pull request #{}: {}", pr.number, pr.url)
            }
            (Some(b), None) => writeln!(out, "Pushed {b}; open a pull request in your Git host."),
        }
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

fn slug(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= max {
            break;
        }
    }
    out.trim_end_matches('-').to_owned()
}

pub fn run(ctx: &Ctx, args: ExportSourceArgs) -> Result<u8> {
    let git = Git::new();
    let w = work::open(ctx, &git, &args.source)?;
    let mut report = ExportSourceReport {
        source: w.name.clone(),
        path: w.dir.display().to_string(),
        branch: None,
        commit: None,
        pushed: false,
        pull_request: None,
        warnings: Vec::new(),
    };
    if args.prepare {
        ctx.emit(&report)?;
        return Ok(exit::OK);
    }
    let status = git.run(Some(&w.dir), ["status", "--porcelain"])?;
    if status.trim().is_empty() {
        bail!(
            "no edits in the working clone of {} ({}); edit items there or in `lo ui` first",
            w.name,
            w.dir.display()
        );
    }
    let base = work::default_branch(&git, &w.dir)?;
    let user = git
        .run(Some(&w.dir), ["config", "user.name"])
        .ok()
        .map(|u| slug(u.trim(), 24))
        .filter(|u| !u.is_empty())
        .or_else(|| {
            std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok()
                .map(|u| slug(&u, 24))
        })
        .unwrap_or_else(|| "user".into());
    let stamp =
        &blake3::hash(format!("{}{:?}", args.message, std::time::SystemTime::now()).as_bytes())
            .to_hex()[..6];
    let branch = args
        .branch
        .clone()
        .unwrap_or_else(|| format!("loadout/{user}/{}-{stamp}", slug(&args.message, 40)));
    if !branch.starts_with("loadout/") {
        bail!("export branches must start with `loadout/` (got {branch:?})");
    }
    if branch == base || branch.trim_start_matches("refs/heads/") == base {
        bail!("refusing to push to the default branch {base:?}");
    }
    git.run(Some(&w.dir), ["checkout", "--quiet", "-b", &branch])
        .with_context(|| format!("creating branch {branch}"))?;
    git.run(Some(&w.dir), ["add", "--all"])?;
    let committed = git.run(
        Some(&w.dir),
        [
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            &args.message,
        ],
    );
    if let Err(e) = committed {
        let _ = git.run(Some(&w.dir), ["checkout", "--quiet", &base]);
        let _ = git.run(Some(&w.dir), ["branch", "-D", &branch]);
        return Err(e).context("committing (is git user.name/user.email set?)");
    }
    let commit = git.head(&w.dir)?;
    // An explicit refspec to a new `loadout/` branch; never --force.
    git.run(
        Some(&w.dir),
        [
            "push",
            "--quiet",
            "origin",
            &format!("HEAD:refs/heads/{branch}"),
        ],
    )
    .with_context(|| format!("pushing {branch}"))?;
    report.pushed = true;
    report.branch = Some(branch.clone());
    report.commit = Some(commit);
    // Back to the default branch so the next edits start from it.
    git.run(Some(&w.dir), ["checkout", "--quiet", &base])?;
    let _ = git.run(Some(&w.dir), ["pull", "--quiet", "--ff-only"]);

    // The URL as configured (not rewritten by `url.<x>.insteadOf`).
    let remote = git.run(Some(&w.dir), ["config", "--get", "remote.origin.url"])?;
    match github_repo(remote.trim()) {
        Some((host, owner, repo)) => match open_pr(
            &git,
            &host,
            &owner,
            &repo,
            &branch,
            &base,
            &args.message,
            &w.name,
        ) {
            Ok(pr) => report.pull_request = Some(pr),
            Err(e) => report.warnings.push(format!(
                "pushed {branch}, but opening the pull request failed: {e:#}"
            )),
        },
        None => report.warnings.push(format!(
            "{} isn't a GitHub repo; open a pull request for {branch} in your Git host",
            remote.trim()
        )),
    }
    let ok = report.pull_request.is_some();
    ctx.emit(&report)?;
    Ok(if ok { exit::OK } else { exit::ERROR })
}

/// `https://github.com/o/r(.git)`, `git@github.com:o/r(.git)`,
/// `ssh://git@github.com/o/r` → (host, owner, repo) when the host is
/// github.com or the configured GitHub API's host.
pub fn github_repo(url: &str) -> Option<(String, String, String)> {
    let rest = if let Some(r) = url.strip_prefix("git@") {
        r.replacen(':', "/", 1)
    } else {
        let r = url.split_once("://")?.1;
        r.rsplit_once('@').map_or(r, |(_, h)| h).to_owned()
    };
    let mut parts = rest.trim_end_matches('/').splitn(3, '/');
    let host = parts.next()?.to_owned();
    let owner = parts.next()?.to_owned();
    let repo = parts.next()?.trim_end_matches(".git").to_owned();
    if repo.is_empty() || repo.contains('/') {
        return None;
    }
    let api_host = std::env::var("LOADOUT_GITHUB_API")
        .ok()
        .map(|a| loadout_members::github::api_host(&a));
    (host == "github.com" || api_host.as_deref() == Some(host.as_str()))
        .then_some((host, owner, repo))
}

#[allow(clippy::too_many_arguments)]
fn open_pr(
    git: &Git,
    host: &str,
    owner: &str,
    repo: &str,
    branch: &str,
    base: &str,
    title: &str,
    source: &str,
) -> Result<PullRequest> {
    let api = std::env::var("LOADOUT_GITHUB_API")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| loadout_members::github::DEFAULT_API.to_owned());
    let env = |k: &str| std::env::var(k).ok();
    let token = loadout_members::github::discover_token(&env, git, host)
        .context("no GitHub token (set GH_TOKEN or run `gh auth login`)")?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .user_agent(concat!("loadout/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let url = format!("{}/repos/{owner}/{repo}/pulls", api.trim_end_matches('/'));
    let body = serde_json::json!({
        "title": title,
        "head": branch,
        "base": base,
        "body": format!("Proposed from Loadout (`lo ui` / `lo export-source {source}`). Review and merge through this repository's usual process."),
        "maintainer_can_modify": true,
    });
    let mut resp = agent
        .post(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("Authorization", format!("Bearer {}", token.expose_secret()))
        .header("Content-Type", "application/json")
        .send(serde_json::to_string(&body)?)
        .with_context(|| format!("POST {url}"))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    if status != 201 {
        bail!(
            "POST {url} returned HTTP {status}: {}",
            text.chars().take(200).collect::<String>()
        );
    }
    let v: serde_json::Value = serde_json::from_str(&text)?;
    Ok(PullRequest {
        url: v["html_url"].as_str().unwrap_or_default().to_owned(),
        number: v["number"].as_u64().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_remotes() {
        let want = Some((
            "github.com".to_owned(),
            "acme".to_owned(),
            "skills".to_owned(),
        ));
        assert_eq!(github_repo("https://github.com/acme/skills.git"), want);
        assert_eq!(github_repo("git@github.com:acme/skills.git"), want);
        assert_eq!(github_repo("ssh://git@github.com/acme/skills"), want);
        assert_eq!(github_repo("https://git.example.com/acme/skills"), None);
        assert_eq!(github_repo("/tmp/local/repo"), None);
    }

    #[test]
    fn slugs() {
        assert_eq!(
            slug("Fix the write-spec skill!", 40),
            "fix-the-write-spec-skill"
        );
        assert_eq!(slug("Ada Lovelace", 24), "ada-lovelace");
    }
}

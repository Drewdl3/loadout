//! Loading the company config and refreshing the profile.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use loadout_git::Git;
use loadout_members::exec::ShellGroups;
use loadout_members::github::LazyGithub;
use loadout_members::gitlab::LazyGitlab;
use loadout_members::repo::GitRepoAccess;
use loadout_members::{Discovery, GroupStatus, Providers, discover, profile_table};
use loadout_model::config::normalize_url;
use loadout_model::manifest::MANIFEST_FILE;
use loadout_model::{CompanyConfig, Config, ManifestDoc, Profile};
use loadout_targets::fsutil::atomic_write;
use serde::{Deserialize, Serialize};

use crate::ctx::Ctx;

/// A fetched and parsed company config repo.
pub struct LoadedCompanyConfig {
    pub url: String,
    pub company_config: CompanyConfig,
    pub commit: String,
    /// True when the fetch failed and the previous checkout was used.
    pub cached: bool,
    pub warning: Option<String>,
}

/// Fetches (or, with `fetch == false`, reuses) the company config checkout and
/// parses its `company:` block.
pub fn load_company_config(
    ctx: &Ctx,
    git: &Git,
    url: &str,
    fetch: bool,
) -> Result<LoadedCompanyConfig> {
    let dir = loadout_git::repo_dir(&ctx.paths.repos_dir(), &normalize_url(url));
    let (commit, cached, warning) = if fetch {
        match git.sync_checkout(url, &dir, None) {
            Ok(c) => (c, false, None),
            Err(e) => match git.head(&dir) {
                Ok(c) => (
                    c,
                    true,
                    Some(format!(
                        "company config {url}: fetch failed, using cached checkout ({e})"
                    )),
                ),
                Err(_) => return Err(e).with_context(|| format!("fetching company config {url}")),
            },
        }
    } else {
        (
            git.head(&dir)
                .with_context(|| format!("no checkout of company config {url}; run `lo sync`"))?,
            true,
            None,
        )
    };
    let text = std::fs::read_to_string(dir.join(MANIFEST_FILE))
        .with_context(|| format!("company config {url} has no {MANIFEST_FILE}"))?;
    let doc = ManifestDoc::parse(&text).with_context(|| format!("company config {url}"))?;
    let Some(block) = &doc.manifest.company_config else {
        bail!(
            "{url} is a source repo, not a company config: its {MANIFEST_FILE} has no `company:` block"
        );
    };
    let company_config =
        CompanyConfig::from_value(block).with_context(|| format!("company config {url}"))?;
    Ok(LoadedCompanyConfig {
        url: url.to_owned(),
        company_config,
        commit,
        cached,
        warning,
    })
}

/// The cached profile from `[profile]`.
pub fn cached_profile(config: &Config) -> Profile {
    let mut p = Profile::default();
    for (layer, groups) in &config.profile {
        for u in &groups.0 {
            p.add(layer.as_str(), u.as_str());
        }
    }
    p
}

/// Runs membership discovery with the real providers.
pub fn run_discovery(company_config: &CompanyConfig, config: &Config) -> Discovery {
    let git = Git::new();
    let env = |k: &str| std::env::var(k).ok();
    let github = LazyGithub::new(&env, &git);
    let gitlab = LazyGitlab::new(&env, &git);
    let exec = ShellGroups::default();
    let repo = GitRepoAccess(&git);
    discover(
        company_config,
        &config.membership,
        &cached_profile(config),
        &Providers {
            env: &env,
            github: &github,
            gitlab: &gitlab,
            exec: &exec,
            repo: &repo,
        },
    )
}

/// `membership.json`: the result of the last discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MembershipState {
    pub version: u32,
    pub company_config: String,
    /// Seconds since the Unix epoch.
    pub refreshed_at: u64,
    pub groups: Vec<GroupStatus>,
}

const STATE_VERSION: u32 = 1;

fn state_path(ctx: &Ctx) -> PathBuf {
    ctx.paths.data_dir.join("membership.json")
}

pub fn load_state(ctx: &Ctx) -> Option<MembershipState> {
    let text = std::fs::read_to_string(state_path(ctx)).ok()?;
    serde_json::from_str::<MembershipState>(&text)
        .ok()
        .filter(|s| s.version == STATE_VERSION)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Whether the profile should be re-discovered: never done for this
/// company config, or older than `profile_refresh_interval`.
pub fn refresh_due(ctx: &Ctx, config: &Config, company_config_url: &str) -> Result<bool> {
    let interval: Duration = config.profile_refresh_interval()?;
    Ok(match load_state(ctx) {
        Some(s) if normalize_url(&s.company_config) == normalize_url(company_config_url) => {
            now_secs().saturating_sub(s.refreshed_at) >= interval.as_secs()
        }
        _ => true,
    })
}

/// Writes the discovered profile to `[profile]` and `membership.json`.
pub fn save_discovery(ctx: &Ctx, company_config_url: &str, d: &Discovery) -> Result<()> {
    let mut doc = ctx.load_config_doc()?;
    doc.set_profile(&profile_table(&d.profile));
    ctx.save_config(&doc)?;
    let state = MembershipState {
        version: STATE_VERSION,
        company_config: company_config_url.to_owned(),
        refreshed_at: now_secs(),
        groups: d.groups.clone(),
    };
    let mut json = serde_json::to_vec_pretty(&state)?;
    json.push(b'\n');
    let path = state_path(ctx);
    atomic_write(&path, &json, false).with_context(|| format!("writing {}", path.display()))
}

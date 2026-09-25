//! Looks for a newer Loadout release, at most once a day, and remembers the
//! answer in `update-check.json`. Only reports: installing is always an
//! explicit `lo self-update`. `check_for_updates = false` in `config.toml`
//! turns the lookup off.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use loadout_targets::fsutil::atomic_write;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::self_update;
use crate::ctx::Ctx;
use crate::membership::now_secs;

/// Where the project lives; shown in the web UI.
pub const REPO_URL: &str = "https://github.com/Drewdl3/loadout";

/// How long a lookup's answer is trusted.
pub const CHECK_EVERY_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cached {
    /// Unix seconds of the last successful lookup.
    pub checked_at: u64,
    /// The latest release's version, without a leading `v`.
    pub latest: String,
}

/// The setting and the cache are about this binary, so they're the user's
/// even in project mode (whose `config.toml` lives in the repo).
fn user_ctx() -> Result<Ctx> {
    Ctx::from_env(false, true)
}

pub fn cache_file(user: &Ctx) -> PathBuf {
    user.paths.data_dir.join("update-check.json")
}

/// Whether the user's `config.toml` allows update lookups.
pub fn enabled() -> Result<bool> {
    Ok(user_ctx()?.load_config()?.check_for_updates())
}

/// Refreshes the cached answer if lookups are enabled and it's stale.
pub fn check_in_background() {
    std::thread::spawn(|| {
        let run = || -> Result<()> {
            if enabled()? {
                refresh_if_stale(&cache_file(&user_ctx()?))?;
            }
            Ok(())
        };
        if let Err(e) = run() {
            tracing::debug!("update check failed: {e:#}");
        }
    });
}

pub fn load(path: &Path) -> Option<Cached> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Whether a lookup made at `checked_at` should be repeated at `now`.
pub fn is_stale(cached: Option<&Cached>, now: u64) -> bool {
    cached.is_none_or(|c| now.saturating_sub(c.checked_at) >= CHECK_EVERY_SECS)
}

/// Asks the releases API for the latest version if the cached answer is
/// missing or stale. Failures leave the cache as it was.
pub fn refresh_if_stale(path: &Path) -> Result<()> {
    if !is_stale(load(path).as_ref(), now_secs()) {
        return Ok(());
    }
    let cached = Cached {
        checked_at: now_secs(),
        latest: self_update::latest_version()?,
    };
    let mut json = serde_json::to_vec_pretty(&cached)?;
    json.push(b'\n');
    atomic_write(path, &json, false).with_context(|| format!("writing {}", path.display()))
}

/// This build's version and repo, plus what the last lookup found.
pub fn about() -> Result<Value> {
    let user = user_ctx()?;
    let enabled = user.load_config()?.check_for_updates();
    let cached = enabled.then(|| load(&cache_file(&user))).flatten();
    let latest = cached.as_ref().map(|c| c.latest.clone());
    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "repo": REPO_URL,
        "check_for_updates": enabled,
        "latest": latest,
        "update_available": latest.as_deref().is_some_and(self_update::is_newer),
        "checked_at": cached.map(|c| loadout_model::time::format_rfc3339(c.checked_at as i64)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_after_a_day() {
        let c = Cached {
            checked_at: 1_000,
            latest: "0.1.0".into(),
        };
        assert!(is_stale(None, 5));
        assert!(!is_stale(Some(&c), 1_000));
        assert!(!is_stale(Some(&c), 1_000 + CHECK_EVERY_SECS - 1));
        assert!(is_stale(Some(&c), 1_000 + CHECK_EVERY_SECS));
    }
}

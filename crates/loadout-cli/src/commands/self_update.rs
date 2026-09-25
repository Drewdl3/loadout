//! `lo self-update [--check] [--version <tag>]`: replace this
//! binary with a GitHub release build, verified against the release's
//! `SHA256SUMS`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;
use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ctx::{Ctx, Report};
use crate::exit;

/// Where releases come from; `LOADOUT_RELEASES_API` overrides it
/// (forks, mirrors, tests).
pub const DEFAULT_RELEASES_API: &str = "https://api.github.com/repos/Drewdl3/loadout";

#[derive(Debug, Args)]
pub struct SelfUpdateArgs {
    /// Only report whether a newer release exists.
    #[arg(long)]
    pub check: bool,
    /// Install this release tag (e.g. v0.1.0) instead of the latest.
    #[arg(long)]
    pub version: Option<String>,
    /// Replace this file instead of the running binary.
    #[arg(long, hide = true)]
    pub path: Option<PathBuf>,
}

/// `--json` output of `lo self-update`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SelfUpdateReport {
    pub current: String,
    /// The release looked at (without the leading `v`).
    pub latest: String,
    pub update_available: bool,
    pub updated: bool,
    pub path: String,
}

impl Report for SelfUpdateReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if self.updated {
            writeln!(
                out,
                "Updated lo {} → {} ({})",
                self.current, self.latest, self.path
            )
        } else if self.update_available {
            writeln!(
                out,
                "lo {} is available (you have {}); run `lo self-update`.",
                self.latest, self.current
            )
        } else {
            writeln!(out, "lo {} is up to date.", self.current)
        }
    }
}

/// The release target this binary was built for.
pub fn target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-musl"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

fn parse_version(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split(['.', '-'])
        .map_while(|p| p.parse().ok())
        .collect()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .http_status_as_error(false)
        .user_agent(concat!("loadout/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn get_bytes(url: &str) -> Result<Vec<u8>> {
    let mut resp = agent()
        .get(url)
        .header("Accept", "application/octet-stream")
        .call()
        .with_context(|| format!("GET {url}"))?;
    let status = resp.status().as_u16();
    if status != 200 {
        bail!("GET {url} returned HTTP {status}");
    }
    Ok(resp
        .body_mut()
        .with_config()
        .limit(256 * 1024 * 1024)
        .read_to_vec()?)
}

pub fn run(ctx: &Ctx, args: SelfUpdateArgs) -> Result<u8> {
    let api = std::env::var("LOADOUT_RELEASES_API")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_RELEASES_API.to_owned());
    let api = api.trim_end_matches('/');
    let url = match &args.version {
        Some(tag) => format!("{api}/releases/tags/{tag}"),
        None => format!("{api}/releases/latest"),
    };
    let release: serde_json::Value = serde_json::from_slice(&get_bytes(&url)?)
        .with_context(|| format!("unexpected response from {url}"))?;
    let tag = release["tag_name"]
        .as_str()
        .context("release has no tag_name")?
        .to_owned();
    let latest = tag.trim_start_matches('v').to_owned();
    let current = env!("CARGO_PKG_VERSION").to_owned();
    let update_available = parse_version(&latest) > parse_version(&current);
    let path = match args.path {
        Some(p) => p,
        None => std::env::current_exe().context("finding the running binary")?,
    };
    let mut report = SelfUpdateReport {
        current,
        latest: latest.clone(),
        update_available,
        updated: false,
        path: path.display().to_string(),
    };
    if args.check || (!update_available && args.version.is_none()) {
        ctx.emit(&report)?;
        return Ok(exit::OK);
    }

    let target = target().context("no release builds exist for this platform")?;
    let assets = release["assets"]
        .as_array()
        .context("release has no assets")?;
    let find = |pred: &dyn Fn(&str) -> bool| {
        assets.iter().find_map(|a| {
            let name = a["name"].as_str()?;
            pred(name).then(|| {
                (
                    name.to_owned(),
                    a["browser_download_url"].as_str().map(str::to_owned),
                )
            })
        })
    };
    let (archive_name, archive_url) = find(&|n: &str| {
        n.starts_with("lo-")
            && n.contains(target)
            && (n.ends_with(".tar.gz") || n.ends_with(".zip"))
    })
    .with_context(|| format!("release {tag} has no build for {target}"))?;
    let archive_url = archive_url.context("asset has no download URL")?;
    let (_, sums_url) = find(&|n: &str| n == "SHA256SUMS").context("release has no SHA256SUMS")?;
    let sums = String::from_utf8(get_bytes(&sums_url.context("SHA256SUMS has no URL")?)?)?;
    let expected = sums
        .lines()
        .find_map(|l| {
            let mut parts = l.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?.trim_start_matches('*');
            (name == archive_name).then(|| hash.to_ascii_lowercase())
        })
        .with_context(|| format!("SHA256SUMS doesn't list {archive_name}"))?;
    let data = get_bytes(&archive_url)?;
    let actual: String = Sha256::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        bail!(
            "checksum mismatch for {archive_name}: expected {expected}, got {actual}; nothing was changed"
        );
    }

    let dir = path.parent().context("binary has no directory")?;
    let work = tempfile::Builder::new()
        .prefix(".loadout-update-")
        .tempdir_in(dir)
        .context("creating a temporary directory next to the binary")?;
    let archive = work.path().join(&archive_name);
    std::fs::write(&archive, &data)?;
    let status = std::process::Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(work.path())
        .status()
        .context("running tar to unpack the release")?;
    if !status.success() {
        bail!("unpacking {archive_name} failed");
    }
    let exe = if cfg!(windows) { "lo.exe" } else { "lo" };
    let new =
        find_file(work.path(), exe).with_context(|| format!("{archive_name} has no {exe}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
    }
    // The new binary must run before it replaces this one.
    let out = std::process::Command::new(&new)
        .arg("--version")
        .output()
        .context("running the downloaded binary")?;
    let says = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() || !says.contains(&latest) {
        bail!(
            "the downloaded binary reports {:?}, not {latest}; nothing was changed",
            says.trim()
        );
    }
    replace(&new, &path)?;
    report.updated = true;
    ctx.emit(&report)?;
    Ok(exit::OK)
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find_file(&p, name) {
                return Some(f);
            }
        } else if e.file_name() == name {
            return Some(p);
        }
    }
    None
}

/// Swaps `new` into `path` (same directory, so renames are atomic). A
/// running Windows executable can be renamed but not overwritten.
fn replace(new: &Path, path: &Path) -> Result<()> {
    let staged = path.with_extension("new");
    std::fs::copy(new, &staged)?;
    if cfg!(windows) {
        let old = path.with_extension("old");
        let _ = std::fs::remove_file(&old);
        if path.exists() {
            std::fs::rename(path, &old)?;
        }
    }
    std::fs::rename(&staged, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare() {
        assert!(parse_version("v0.10.0") > parse_version("0.9.9"));
        assert!(parse_version("1.0.0") == parse_version("v1.0.0"));
        assert!(parse_version("0.1.1-rc1") > parse_version("0.1.0"));
        assert!(target().is_some());
    }
}

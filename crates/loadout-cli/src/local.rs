//! The local source: a Git repo in Loadout's data folder holding skills
//! you manage on this computer that don't come from anywhere else. It's an
//! ordinary source at the `user` layer, subscribed and auto-applied, so
//! everything Loadout does for shared items (toggles, every tool, search,
//! the editor) works for these too. Its edits are committed in place, never
//! pushed or sent as pull requests.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use loadout_git::Git;
use loadout_model::SourceSub;
use loadout_model::config::normalize_url;
use loadout_model::manifest::MANIFEST_FILE;

use crate::ctx::Ctx;
use crate::paths::Paths;

/// The local source's name (in its `LOADOUT.md`).
pub const NAME: &str = "local-skills";
/// How it's labelled in the UI.
pub const LABEL: &str = "This computer";
/// Ties with other `user`-layer sources go to your own copy.
const PRIORITY: i64 = 10;

/// `<data>/local`.
pub fn dir(paths: &Paths) -> PathBuf {
    paths.data_dir.join("local")
}

/// The URL it's subscribed with.
pub fn url(paths: &Paths) -> String {
    dir(paths).display().to_string()
}

/// Whether `url` is the local source.
pub fn is_local(paths: &Paths, url: &str) -> bool {
    paths.project.is_none() && normalize_url(url) == normalize_url(&self::url(paths))
}

/// Creates the local source (once) and subscribes to it, its updates
/// applying without review. Returns its folder.
pub fn ensure(ctx: &Ctx, git: &Git) -> Result<PathBuf> {
    if ctx.paths.project.is_some() {
        bail!("your own skills live in your user setup; run this outside a project");
    }
    let dir = dir(&ctx.paths);
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(dir.join("skills"))?;
        std::fs::write(
            dir.join(MANIFEST_FILE),
            format!(
                "---\nloadout: 1\nname: {NAME}\nlayer: user\ndescription: Skills you manage on this computer.\n---\n\n\
                 # Skills on this computer\n\nManaged by Loadout (`lo skills`). Edit them in `lo ui`, or here and run `lo sync`.\n"
            ),
        )?;
        std::fs::write(dir.join("skills/.gitkeep"), "")?;
        git.run(Some(&dir), ["init", "--quiet"])
            .context("creating the local source")?;
        commit(git, &dir, "Start local skills")?;
    }
    let mut doc = ctx.load_config_doc()?;
    let url = url(&ctx.paths);
    let mut changed = doc.add_source(&SourceSub {
        label: Some(LABEL.to_owned()),
        priority: PRIORITY,
        ..SourceSub::new(url.clone())
    });
    changed |= doc.add_auto_apply(&url);
    if changed {
        ctx.save_config(&doc)?;
    }
    Ok(dir)
}

/// Commits everything in the local source. Returns the new commit, or
/// `None` when there was nothing to commit.
pub fn commit(git: &Git, dir: &Path, message: &str) -> Result<Option<String>> {
    git.run(Some(dir), ["add", "--all"])?;
    if git
        .run(Some(dir), ["status", "--porcelain"])?
        .trim()
        .is_empty()
    {
        return Ok(None);
    }
    // Your Git identity when you have one; the commits never leave this
    // computer, so a placeholder is fine otherwise.
    let has_identity = git.run(Some(dir), ["config", "user.email"]).is_ok();
    let mut args: Vec<&str> = Vec::new();
    if !has_identity {
        args.extend([
            "-c",
            "user.name=Loadout",
            "-c",
            "user.email=loadout@localhost",
        ]);
    }
    args.extend([
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--quiet",
        "--no-verify",
        "-m",
        message,
    ]);
    git.run(Some(dir), args)?;
    Ok(Some(
        git.run(Some(dir), ["rev-parse", "HEAD"])?.trim().to_owned(),
    ))
}

/// Installs what the local source now holds: a sync that doesn't refresh
/// group membership.
pub fn apply(ctx: &Ctx) -> Result<crate::engine::ApplyReport> {
    let config = ctx.load_config()?;
    crate::engine::apply(
        ctx,
        &config,
        crate::engine::Options::new(crate::engine::Fetch::Remote),
    )
}

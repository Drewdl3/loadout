//! Working clones for editing sources: full clones under
//! `data/work/`, separate from the read-only store clones, where the UI
//! (or the user) edits items before `lo export-source` turns the edits
//! into a branch and a pull request.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use loadout_core::resolve::EnabledBy;
use loadout_git::Git;
use loadout_model::config::normalize_url;
use loadout_model::manifest::MANIFEST_FILE;
use loadout_model::{ItemKey, ItemKind, LayerModel, ManifestDoc};

use crate::ctx::Ctx;
use crate::membership;
use crate::state::Resolved;

/// A source the user can edit.
pub struct Work {
    pub name: String,
    pub url: String,
    pub dir: PathBuf,
}

/// `data/work/<blake3(url)[:16]>`.
pub fn work_dir(ctx: &Ctx, url: &str) -> PathBuf {
    loadout_git::repo_dir(&ctx.paths.data_dir.join("work"), &normalize_url(url))
}

/// The working clone of the source named `name` (cloned on first use; an
/// existing clone is left as is so local edits survive).
pub fn open(ctx: &Ctx, git: &Git, name: &str) -> Result<Work> {
    let Some(resolved) = Resolved::load(&ctx.paths.resolved_file())? else {
        bail!("nothing synced yet; run `lo sync`");
    };
    let Some(src) = resolved.sources.iter().find(|s| s.name == name) else {
        bail!("no source named {name:?} (see `lo info` / `lo list`)");
    };
    let dir = work_dir(ctx, &src.url);
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(dir.parent().expect("parent"))?;
        git.run(
            None,
            [
                "clone".as_ref(),
                "--quiet".as_ref(),
                src.url.as_ref(),
                dir.as_os_str(),
            ],
        )
        .with_context(|| format!("cloning {} for editing", src.url))?;
    }
    Ok(Work {
        name: name.to_owned(),
        url: src.url.clone(),
        dir,
    })
}

/// The repo-relative location of `key` in a source: (path of the item, is a
/// directory, path of its main file).
pub fn item_path(dir: &Path, key: &ItemKey) -> Result<(String, bool, String)> {
    let text = std::fs::read_to_string(dir.join(MANIFEST_FILE))
        .with_context(|| format!("{} has no {MANIFEST_FILE}", dir.display()))?;
    let paths = ManifestDoc::parse(&text)?.manifest.paths;
    let base = paths.for_kind(key.kind()).to_owned();
    let name = key.name();
    Ok(match key.kind() {
        ItemKind::Skill => (
            format!("{base}/{name}"),
            true,
            format!("{base}/{name}/SKILL.md"),
        ),
        ItemKind::Plugin => (
            format!("{base}/{name}"),
            true,
            format!("{base}/{name}/PLUGIN.md"),
        ),
        ItemKind::Extra => {
            let (ty, n) = name
                .split_once('.')
                .context("extra names are <type>.<name>")?;
            let p = format!("{base}/{ty}/{n}.md");
            (p.clone(), false, p)
        }
        ItemKind::Mcp | ItemKind::Agent => {
            let p = format!("{base}/{name}.md");
            (p.clone(), false, p)
        }
    })
}

/// A template's main file in a source checkout: `<templates>/<name>/SKILL.md`.
pub fn template_path(dir: &Path, name: &str) -> Result<String> {
    loadout_model::id::validate_item_name(name)?;
    let text = std::fs::read_to_string(dir.join(MANIFEST_FILE))
        .with_context(|| format!("{} has no {MANIFEST_FILE}", dir.display()))?;
    let paths = ManifestDoc::parse(&text)?.manifest.paths;
    Ok(format!("{}/{name}/SKILL.md", paths.templates()))
}

/// Joins a `/`-separated repo path.
pub fn join(dir: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(dir.to_path_buf(), |p, c| p.join(c))
}

/// Refuses edits that a higher layer has locked or made required:
/// a lower-layer source can't provide or change such an item.
pub fn check_editable(ctx: &Ctx, source_dir: &Path, key: &ItemKey) -> Result<()> {
    let Some(resolved) = Resolved::load(&ctx.paths.resolved_file())? else {
        return Ok(());
    };
    let Some(entry) = resolved.resolution.get(key) else {
        return Ok(());
    };
    let text = std::fs::read_to_string(source_dir.join(MANIFEST_FILE))?;
    let manifest = ManifestDoc::parse(&text)?.manifest;
    let layers = match ctx.load_config()?.company_config {
        Some(url) if ctx.paths.project.is_none() => {
            membership::load_company_config(ctx, &Git::new(), &url, false)
                .map(|c| c.company_config.layer_model())
                .unwrap_or_default()
        }
        _ => LayerModel::default(),
    };
    let my_rank = layers.rank(&manifest.layer).unwrap_or(i64::MAX);
    let Some(winner) = &entry.winner else {
        return Ok(());
    };
    if winner.source() == manifest.name {
        return Ok(());
    }
    let Some(trail) = entry.candidates.iter().find(|c| &c.id == winner) else {
        return Ok(());
    };
    let binding =
        trail.locked || !trail.overridable || entry.enabled_by == Some(EnabledBy::Required);
    if binding && trail.rank.is_some_and(|r| r < my_rank) {
        bail!(
            "{key} is {} by {winner} ({} layer); it can't be edited from {} ({} layer)",
            if trail.locked || !trail.overridable {
                "locked"
            } else {
                "required"
            },
            trail.layer,
            manifest.name,
            manifest.layer
        );
    }
    Ok(())
}

/// The branch the working clone's `origin` publishes by default.
pub fn default_branch(git: &Git, dir: &Path) -> Result<String> {
    if let Ok(out) = git.run(
        Some(dir),
        ["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        let b = out.trim().trim_start_matches("origin/").to_owned();
        if !b.is_empty() {
            return Ok(b);
        }
    }
    let out = git.run(Some(dir), ["rev-parse", "--abbrev-ref", "HEAD"])?;
    Ok(out.trim().to_owned())
}

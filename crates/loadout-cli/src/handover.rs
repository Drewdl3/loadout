//! Handing adopted skills over to Loadout.
//!
//! `lo adopt` copies a skill from an AI tool's folder into a source, but the
//! original stays where it was. Once the source ships the skill, Loadout
//! installs its own copy, and tools that read several folders would see
//! both. So `lo adopt` records each adoption in `adopted.json`, and
//! `lo sync` moves the original (and the links other tools had to it) into
//! a backup once the source ships the skill. `lo adopt --restore` puts it
//! back.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use loadout_targets::fsutil::atomic_write;
use loadout_targets::{TargetTable, expand_path};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

const VERSION: u32 = 1;

/// `adopted.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdoptedFile {
    pub version: u32,
    #[serde(default)]
    pub skills: Vec<Adoption>,
}

/// A skill copied into a source from a tool's skills folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adoption {
    /// Its name in the source (it may have been renamed on the way).
    pub name: String,
    pub source: String,
    /// The folder it was copied from.
    pub original: PathBuf,
    /// `blake3:<hex>` of the original when it was adopted. An original
    /// changed since then is left alone.
    pub content_hash: String,
    /// Where the original was moved, once handed over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<PathBuf>,
    /// Links to the original that were removed when it was handed over.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<RemovedLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedLink {
    pub path: PathBuf,
    /// The link's target as it was written (it may be relative).
    pub target: PathBuf,
}

/// An original moved aside by `lo sync` (or that would be, with
/// `--dry-run`).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct HandedOver {
    /// `skill/<name>`.
    pub item: String,
    pub source: String,
    /// The original folder.
    pub from: String,
    /// Where it was moved.
    pub backup: String,
    /// Links to it, in other tools' folders, that were removed.
    pub links: Vec<String>,
}

impl AdoptedFile {
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let file: Self = serde_json::from_str(&text)
            .with_context(|| format!("{} is corrupt", path.display()))?;
        if file.version > VERSION {
            bail!("{} was written by a newer Loadout", path.display());
        }
        Ok(file)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut json = serde_json::to_vec_pretty(&AdoptedFile {
            version: VERSION,
            skills: self.skills.clone(),
        })?;
        json.push(b'\n');
        atomic_write(path, &json, false).with_context(|| format!("writing {}", path.display()))
    }

    /// Records an adoption, replacing a pending one of the same original.
    pub fn record(&mut self, adoption: Adoption) {
        self.skills
            .retain(|a| a.backup.is_some() || a.original != adoption.original);
        self.skills.push(adoption);
    }

    /// The pending (not yet handed over) adoption of the folder at `path`.
    pub fn pending(&self, path: &Path) -> Option<&Adoption> {
        self.skills
            .iter()
            .find(|a| a.backup.is_none() && a.original == path)
    }
}

/// `blake3:<hex>` over a skill folder's files.
pub fn folder_hash(dir: &Path) -> Result<String> {
    let (files, _) = crate::scan::read_tree(dir)?;
    Ok(loadout_core::hash::tree_hash(
        files
            .iter()
            .map(|f| (f.path.as_str(), f.contents.as_slice())),
    ))
}

/// Every tool's skills folder (enabled or not), each once.
fn skills_dirs(paths: &Paths) -> Result<Vec<PathBuf>> {
    let table = TargetTable::load(&paths.user_targets_dir())?;
    let mut seen = BTreeSet::new();
    Ok(table
        .iter()
        .filter_map(|t| {
            t.skills
                .as_ref()
                .and_then(|s| expand_path(&s.path, &paths.home))
        })
        .filter(|d| seen.insert(d.clone()))
        .collect())
}

/// Links in the tools' skills folders that point at `original`.
fn links_to(dirs: &[PathBuf], original: &Path) -> Vec<RemovedLink> {
    let Ok(real) = original.canonicalize() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in entries.filter_map(|e| e.ok()) {
            let path = e.path();
            let Ok(target) = std::fs::read_link(&path) else {
                continue;
            };
            if path.canonicalize().is_ok_and(|p| p == real) {
                out.push(RemovedLink { path, target });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// `<data>/adopted/<name>`, or `<name>-2`, … when that's taken.
fn backup_dir(paths: &Paths, name: &str) -> PathBuf {
    let base = paths.data_dir.join("adopted");
    let mut dir = base.join(name);
    let mut n = 2;
    while std::fs::symlink_metadata(&dir).is_ok() {
        dir = base.join(format!("{name}-{n}"));
        n += 1;
    }
    dir
}

/// Moves a folder, copying when a rename can't (another filesystem).
fn move_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to.parent().expect("has parent"))?;
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let (files, _) = crate::scan::read_tree(from)?;
    for f in &files {
        let path = crate::work::join(to, &f.path);
        std::fs::create_dir_all(path.parent().expect("has parent"))?;
        std::fs::write(&path, &f.contents)?;
    }
    std::fs::remove_dir_all(from).with_context(|| format!("removing {}", from.display()))
}

/// Hands over every pending adoption whose skill Loadout now installs
/// (`ships(source, name)`): its original moves to a backup, and links to
/// it in the tools' folders are removed. An original changed since it was
/// adopted is left alone, with a warning.
pub fn hand_over(
    paths: &Paths,
    ships: impl Fn(&str, &str) -> bool,
    dry_run: bool,
    warnings: &mut Vec<String>,
) -> Result<Vec<HandedOver>> {
    let file_path = paths.adopted_file();
    let mut file = AdoptedFile::load(&file_path)?;
    if !file.skills.iter().any(|a| a.backup.is_none()) {
        return Ok(Vec::new());
    }
    let dirs = skills_dirs(paths)?;
    let mut out = Vec::new();
    let mut changed = false;
    let mut kept = Vec::new();
    for mut a in std::mem::take(&mut file.skills) {
        if a.backup.is_some() || !ships(&a.source, &a.name) {
            kept.push(a);
            continue;
        }
        let is_dir = std::fs::symlink_metadata(&a.original).is_ok_and(|m| m.file_type().is_dir());
        if !is_dir {
            // Gone, or replaced by something else: nothing to hand over.
            changed = true;
            continue;
        }
        if folder_hash(&a.original).ok().as_deref() != Some(a.content_hash.as_str()) {
            warnings.push(format!(
                "{} changed since you adopted it into {}; left in place. Move it aside yourself so the shared skill/{} isn't installed twice.",
                a.original.display(),
                a.source,
                a.name
            ));
            kept.push(a);
            continue;
        }
        let backup = backup_dir(paths, &a.name);
        let links = links_to(&dirs, &a.original);
        out.push(HandedOver {
            item: format!("skill/{}", a.name),
            source: a.source.clone(),
            from: a.original.display().to_string(),
            backup: backup.display().to_string(),
            links: links.iter().map(|l| l.path.display().to_string()).collect(),
        });
        if dry_run {
            kept.push(a);
            continue;
        }
        for l in &links {
            loadout_targets::link::remove(&l.path)
                .with_context(|| format!("removing {}", l.path.display()))?;
        }
        move_dir(&a.original, &backup)
            .with_context(|| format!("moving {} aside", a.original.display()))?;
        a.backup = Some(backup);
        a.links = links;
        kept.push(a);
        changed = true;
    }
    file.skills = kept;
    if changed && !dry_run {
        file.save(&file_path)?;
    }
    Ok(out)
}

/// An original that `lo adopt --restore` put back.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Restored {
    pub name: String,
    /// The folder it was put back in.
    pub path: String,
    /// Links re-created.
    pub links: Vec<String>,
}

/// Puts every handed-over original of `name` (and their links) back.
pub fn restore(paths: &Paths, name: &str, warnings: &mut Vec<String>) -> Result<Vec<Restored>> {
    let file_path = paths.adopted_file();
    let mut file = AdoptedFile::load(&file_path)?;
    let moved: Vec<Adoption> = file
        .skills
        .iter()
        .filter(|a| a.name == name && a.backup.is_some())
        .cloned()
        .collect();
    if moved.is_empty() {
        bail!("no adopted skill named {name:?} was moved aside");
    }
    if let Some(a) = moved
        .iter()
        .find(|a| std::fs::symlink_metadata(&a.original).is_ok())
    {
        bail!(
            "{} is in use (Loadout may have installed the shared skill/{name} there); \
             `lo disable` it or move it first",
            a.original.display()
        );
    }
    let mut out = Vec::new();
    for a in moved {
        let backup = a.backup.as_ref().expect("filtered");
        move_dir(backup, &a.original)
            .with_context(|| format!("restoring {}", a.original.display()))?;
        let mut links = Vec::new();
        for l in &a.links {
            if std::fs::symlink_metadata(&l.path).is_ok() {
                warnings.push(format!(
                    "{} is in use; didn't re-create its link",
                    l.path.display()
                ));
                continue;
            }
            match relink(&l.target, &l.path) {
                Ok(()) => links.push(l.path.display().to_string()),
                Err(e) => warnings.push(format!("re-linking {}: {e}", l.path.display())),
            }
        }
        file.skills.retain(|x| x != &a);
        out.push(Restored {
            name: name.to_owned(),
            path: a.original.display().to_string(),
            links,
        });
    }
    file.save(&file_path)?;
    Ok(out)
}

#[cfg(unix)]
fn relink(target: &Path, at: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}

#[cfg(windows)]
fn relink(target: &Path, at: &Path) -> std::io::Result<()> {
    let target = at.parent().map_or(target.to_path_buf(), |p| p.join(target));
    loadout_targets::link::place(&target, at, loadout_model::LinkMode::Symlink)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adoption(original: &str, backup: Option<&str>) -> Adoption {
        Adoption {
            name: "tdd".into(),
            source: "team".into(),
            original: original.into(),
            content_hash: "blake3:x".into(),
            backup: backup.map(PathBuf::from),
            links: Vec::new(),
        }
    }

    #[test]
    fn recording_again_replaces_only_the_pending_entry() {
        let mut f = AdoptedFile::default();
        f.record(adoption("/h/a/tdd", Some("/d/adopted/tdd")));
        f.record(adoption("/h/a/tdd", None));
        f.record(adoption("/h/a/tdd", None));
        assert_eq!(f.skills.len(), 2);
        assert!(f.pending(Path::new("/h/a/tdd")).is_some());
        assert!(f.pending(Path::new("/h/b/tdd")).is_none());
    }
}

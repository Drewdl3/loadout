//! Working clones for editing sources: full clones under
//! `data/work/`, separate from the read-only store clones, where the UI
//! (or the user) edits items before `lo export-source` turns the edits
//! into a branch and a pull request.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use loadout_core::resolve::EnabledBy;
use loadout_git::Git;
use loadout_model::config::normalize_url;
use loadout_model::manifest::{MANIFEST_FILE, SourcePaths};
use loadout_model::{ItemKey, ItemKind, ManifestDoc};

use crate::ctx::Ctx;
use crate::state::Resolved;

/// A source the user can edit.
pub struct Work {
    pub name: String,
    pub url: String,
    /// The working clone of the source's repo (where Git runs).
    pub root: PathBuf,
    /// The source's directory in it: `root`, or a nested source's folder.
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
    let root = work_dir(ctx, &src.url);
    if !root.join(".git").exists() {
        std::fs::create_dir_all(root.parent().expect("parent"))?;
        git.run(
            None,
            [
                "clone".as_ref(),
                "--quiet".as_ref(),
                src.url.as_ref(),
                root.as_os_str(),
            ],
        )
        .with_context(|| format!("cloning {} for editing", src.url))?;
    }
    Ok(Work {
        name: name.to_owned(),
        url: src.url.clone(),
        dir: src.dir_in(&root),
        root,
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
    // Locks hold at declared ranks, whatever anyone re-ranked locally.
    let (layers, _) = crate::layers::current(ctx)?;
    let my_rank = layers.declared_rank(&manifest.layer).unwrap_or(i64::MAX);
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
    let their_rank = trail.lock_rank.or(trail.rank);
    if binding && their_rank.is_some_and(|r| r < my_rank) {
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

/// How a file, item or template differs from the working clone's last commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Added,
    Modified,
    Removed,
}

impl Change {
    pub fn as_str(self) -> &'static str {
        match self {
            Change::Added => "added",
            Change::Modified => "modified",
            Change::Removed => "removed",
        }
    }

    /// An item whose files changed in different ways was modified.
    fn merge(self, other: Change) -> Change {
        if self == other {
            self
        } else {
            Change::Modified
        }
    }
}

/// The uncommitted edits in a working clone, grouped by what they belong to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub items: BTreeMap<ItemKey, Change>,
    pub templates: BTreeMap<String, Change>,
    /// Changed files outside any item or template (e.g. `LOADOUT.md`).
    pub other: BTreeMap<String, Change>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.templates.is_empty() && self.other.is_empty()
    }
}

/// The edits in the working clone at `root` that belong to one source:
/// the files under `path` (a nested source's directory; `None` for the
/// repo root) minus those under `skip` (the directories of the sources
/// nested inside it). All paths are `/`-separated and repo-relative.
pub fn changes(git: &Git, root: &Path, path: Option<&str>, skip: &[&str]) -> Result<Changes> {
    let status = git.run(
        Some(root),
        ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let dir = path.map_or_else(|| root.to_path_buf(), |p| join(root, p));
    let text = std::fs::read_to_string(dir.join(MANIFEST_FILE))
        .with_context(|| format!("{} has no {MANIFEST_FILE}", dir.display()))?;
    let paths = ManifestDoc::parse(&text)?.manifest.paths;
    let files: Vec<(Change, String)> = parse_status(&status)
        .into_iter()
        .filter(|(_, f)| !skip.iter().any(|s| under(f, s)))
        .filter_map(|(c, f)| match path {
            Some(p) => Some((c, f.strip_prefix(p)?.strip_prefix('/')?.to_owned())),
            None => Some((c, f)),
        })
        .collect();
    Ok(classify(&paths, &files))
}

/// Whether repo path `file` is inside directory `dir`.
fn under(file: &str, dir: &str) -> bool {
    file.strip_prefix(dir)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// `git status --porcelain=v1 -z` → (change, repo path). A rename is its new
/// path added and its old path removed.
fn parse_status(out: &str) -> Vec<(Change, String)> {
    let mut files = Vec::new();
    let mut records = out.split('\0').filter(|r| r.len() > 3);
    while let Some(r) = records.next() {
        let (xy, path) = r.split_at(3);
        let (x, y) = (xy.as_bytes()[0], xy.as_bytes()[1]);
        let change = match (x, y) {
            (b'?', _) | (b'A', _) => Change::Added,
            (b'D', _) | (_, b'D') => Change::Removed,
            (b'R' | b'C', _) => {
                if let Some(from) = records.next().filter(|_| x == b'R') {
                    files.push((Change::Removed, from.to_owned()));
                }
                Change::Added
            }
            _ => Change::Modified,
        };
        files.push((change, path.to_owned()));
    }
    files
}

/// Groups changed files by the item or template they're part of, using the
/// source's layout (`LOADOUT.md` `paths`).
fn classify(paths: &SourcePaths, files: &[(Change, String)]) -> Changes {
    // Most specific directory first, in case one is nested in another.
    let mut dirs: Vec<(&str, Option<ItemKind>)> = ItemKind::ALL
        .into_iter()
        .map(|k| (paths.for_kind(k), Some(k)))
        .chain([(paths.templates(), None)])
        .collect();
    dirs.sort_by_key(|(d, _)| std::cmp::Reverse(d.len()));
    let mut out = Changes::default();
    for (change, file) in files {
        let owner = dirs.iter().find_map(|(dir, kind)| {
            let rest = file.strip_prefix(dir)?.strip_prefix('/')?;
            let mut parts = rest.split('/');
            let first = parts.next()?;
            let name = match kind {
                None | Some(ItemKind::Skill | ItemKind::Plugin) => {
                    parts.next()?;
                    first.to_owned()
                }
                Some(ItemKind::Mcp | ItemKind::Agent) => match parts.next() {
                    None => first.strip_suffix(".md")?.to_owned(),
                    Some(_) => return None,
                },
                Some(ItemKind::Extra) => match (parts.next(), parts.next()) {
                    (Some(n), None) => format!("{first}.{}", n.strip_suffix(".md")?),
                    _ => return None,
                },
            };
            Some(match kind {
                Some(k) => ItemKey::new(*k, name).ok().map(Ok),
                None => Some(Err(name)),
            })
        });
        let entry = match owner.flatten() {
            Some(Ok(key)) => out.items.entry(key),
            Some(Err(template)) => {
                let e = out.templates.entry(template).or_insert(*change);
                *e = e.merge(*change);
                continue;
            }
            None => {
                out.other.insert(file.clone(), *change);
                continue;
            }
        };
        let e = entry.or_insert(*change);
        *e = e.merge(*change);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(yaml: &str) -> SourcePaths {
        let text = format!("---\nloadout: 1\nname: s\nlayer: team\n{yaml}---\n");
        ManifestDoc::parse(&text).unwrap().manifest.paths
    }

    #[test]
    fn status_records_become_files() {
        let out = "?? skills/new/SKILL.md\0 M mcp/jira.md\0 D agents/old.md\0R  skills/b/SKILL.md\0skills/a/SKILL.md\0";
        assert_eq!(
            parse_status(out),
            [
                (Change::Added, "skills/new/SKILL.md".to_owned()),
                (Change::Modified, "mcp/jira.md".to_owned()),
                (Change::Removed, "agents/old.md".to_owned()),
                (Change::Removed, "skills/a/SKILL.md".to_owned()),
                (Change::Added, "skills/b/SKILL.md".to_owned()),
            ]
        );
    }

    #[test]
    fn files_are_grouped_by_item_and_template() {
        let files = [
            (Change::Added, "skills/new/SKILL.md"),
            (Change::Added, "skills/new/scripts/run.sh"),
            (Change::Added, "skills/tweak/extra.md"),
            (Change::Modified, "skills/tweak/SKILL.md"),
            (Change::Removed, "agents/old.md"),
            (Change::Modified, "extras/commands/deploy.md"),
            (Change::Added, "templates/pr-flow/SKILL.md"),
            (Change::Modified, "LOADOUT.md"),
            (Change::Added, "skills/loose.md"),
        ]
        .map(|(c, f)| (c, f.to_owned()));
        let got = classify(&layout(""), &files);
        let items: Vec<_> = got.items.iter().map(|(k, c)| (k.to_string(), *c)).collect();
        assert_eq!(
            items,
            [
                ("skill/new".to_owned(), Change::Added),
                ("skill/tweak".to_owned(), Change::Modified),
                ("agent/old".to_owned(), Change::Removed),
                ("extra/commands.deploy".to_owned(), Change::Modified),
            ]
        );
        assert_eq!(got.templates["pr-flow"], Change::Added);
        assert_eq!(
            got.other.keys().collect::<Vec<_>>(),
            ["LOADOUT.md", "skills/loose.md"]
        );
    }

    #[test]
    fn custom_and_nested_layouts() {
        let paths = layout("paths:\n  skills: ai\n  templates: ai/templates\n");
        let files = [
            (Change::Added, "ai/templates/t/SKILL.md"),
            (Change::Added, "ai/s/SKILL.md"),
        ]
        .map(|(c, f)| (c, f.to_owned()));
        let got = classify(&paths, &files);
        assert_eq!(got.templates["t"], Change::Added);
        assert_eq!(
            got.items
                .keys()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["skill/s"]
        );
    }
}

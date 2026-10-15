//! Every skill on this computer: in each AI tool's skills folder, globally
//! and in projects, whether Loadout put it there or not. Folders reached
//! through links count once, so a skill `npx skills` keeps in
//! `~/.agents/skills` and links into other tools is one copy seen by
//! several tools.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use loadout_model::frontmatter::Document;
use loadout_model::{ItemKey, ItemKind, ItemMeta};
use loadout_targets::owned::OwnedFile;
use loadout_targets::{TargetTable, expand_path};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ctx::Ctx;
use crate::handover::AdoptedFile;
use crate::scan::SKILL_FILE;
use crate::state::Resolved;

/// `lo skills --json`.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct Inventory {
    /// Your home folder, so paths can be shown as `~/…`.
    pub home: String,
    /// By name.
    pub skills: Vec<Skill>,
    /// The folders that were looked in.
    pub folders: Vec<Folder>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// A tool's own folder in your home directory.
    Global,
    /// A project's folder (e.g. `<repo>/.claude/skills`).
    Project,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Folder {
    /// Target id (`claude-code`, …).
    pub tool: String,
    pub scope: Scope,
    pub path: String,
    pub exists: bool,
    /// Loadout installs into this tool.
    pub enabled: bool,
}

/// Where a skill stands overall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    /// Every copy comes from Loadout.
    Managed,
    /// Loadout installs it, but other copies are around too.
    PartlyManaged,
    /// Copied into a source, waiting for it to ship.
    Adopted,
    /// No copy comes from Loadout.
    Unmanaged,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Skill {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub state: State,
    /// The item Loadout installs under this name (`source:skill/name`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// Loadout moved your originals aside when it took this skill over;
    /// `lo skills unmanage` puts them back.
    pub handed_over: bool,
    /// Each distinct folder: Loadout's copy, then the others.
    pub copies: Vec<Copy>,
    /// Things worth fixing, most important first.
    pub issues: Vec<Issue>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Copy {
    /// The real folder (links followed).
    pub path: String,
    pub scope: Scope,
    /// Loadout installed it.
    pub managed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// Copied into this source with `lo adopt`, not shipped yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adopted_into: Option<String>,
    /// Who installed it, when another tool recorded that.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub content_hash: String,
    /// The entries in the tools' folders that reach it.
    pub seen_by: Vec<Sighting>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Sighting {
    pub tool: String,
    pub scope: Scope,
    /// The entry in the tool's folder.
    pub path: String,
    /// It's a link to the copy.
    pub link: bool,
}

/// Where an unmanaged skill came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Origin {
    /// What installed it (`npx skills`).
    pub installer: String,
    /// As the installer names it, e.g. `acme/skills`.
    pub source: String,
    /// A Git URL Loadout can follow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The skill's `SKILL.md` in that repo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Issue {
    /// Something Loadout doesn't manage sits where it wants to install
    /// this item, so these tools don't get Loadout's version.
    Blocked { item: String, tools: Vec<String> },
    /// One tool sees several copies (e.g. a global and a project one).
    Shadowed { tools: Vec<String> },
    /// More than one copy on this computer; `differ` when their content
    /// isn't the same.
    Copies { count: usize, differ: bool },
    /// Not managed, and these tools you use don't have it.
    Missing { tools: Vec<String> },
}

/// What to look at besides the tools' global folders.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Project folders (repos) to look in too.
    pub projects: Vec<PathBuf>,
}

/// A sighting before it's grouped: the entry and what it reaches.
struct Entry {
    name: String,
    tool: String,
    scope: Scope,
    path: PathBuf,
    link: bool,
    real: PathBuf,
    /// The Loadout item installed at `path`.
    owned: Option<ItemKey>,
    /// The project it was found in.
    project: Option<PathBuf>,
}

pub fn scan(ctx: &Ctx, opts: &Options) -> Result<Inventory> {
    let paths = &ctx.paths;
    let table = TargetTable::load(&paths.user_targets_dir())?;
    let config = ctx.load_config()?;
    let (enabled, _) = crate::engine::enabled_targets(&table, &config, &paths.home);
    let enabled: BTreeSet<String> = enabled.iter().map(|t| t.id.clone()).collect();
    let owned = OwnedFile::load(&paths.owned_file()).unwrap_or_default();
    let owned_at: BTreeMap<PathBuf, ItemKey> = owned
        .targets
        .values()
        .flatten()
        .map(|o| (o.dest.clone(), o.item.clone()))
        .collect();
    let resolved = Resolved::load(&paths.resolved_file())?.unwrap_or_default();
    let adopted = AdoptedFile::load(&paths.adopted_file())?;
    let mut out = Inventory {
        home: paths.home.display().to_string(),
        ..Inventory::default()
    };

    // Every folder to look in, each once.
    let mut dirs: Vec<(String, Scope, PathBuf, Option<PathBuf>)> = Vec::new();
    for t in table.iter() {
        let Some(spec) = &t.skills else { continue };
        if let Some(dir) = expand_path(&spec.path, &paths.home) {
            dirs.push((t.id.clone(), Scope::Global, dir, None));
        }
        if let Some(rel) = &spec.project {
            for p in &opts.projects {
                dirs.push((
                    t.id.clone(),
                    Scope::Project,
                    crate::work::join(p, rel),
                    Some(p.clone()),
                ));
            }
        }
    }
    let mut entries = Vec::new();
    let mut seen_dirs = BTreeSet::new();
    for (tool, scope, dir, project) in &dirs {
        out.folders.push(Folder {
            tool: tool.clone(),
            scope: *scope,
            path: dir.display().to_string(),
            exists: dir.is_dir(),
            enabled: enabled.contains(tool),
        });
        // Two tools sharing a folder see the same entries.
        let key = (dir.clone(), tool.clone());
        if !seen_dirs.insert(key) {
            continue;
        }
        let Ok(read) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut names: Vec<_> = read.filter_map(|e| e.ok()).collect();
        names.sort_by_key(|e| e.file_name());
        for e in names {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = e.path();
            let link = std::fs::read_link(&path).is_ok();
            let Ok(real) = path.canonicalize() else {
                if link {
                    out.warnings
                        .push(format!("{}: a broken link", path.display()));
                }
                continue;
            };
            if !real.join(SKILL_FILE).is_file() {
                continue;
            }
            entries.push(Entry {
                name,
                tool: tool.clone(),
                scope: *scope,
                owned: owned_at.get(&path).cloned(),
                path,
                link,
                real,
                project: project.clone(),
            });
        }
    }

    let global_lock = global_lock(ctx);
    let project_locks: BTreeMap<PathBuf, BTreeMap<String, Origin>> = opts
        .projects
        .iter()
        .map(|p| (p.clone(), read_lock(&p.join("skills-lock.json"))))
        .collect();

    // Group by name, then by copy: Loadout's by item, others by folder.
    let mut by_name: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    for e in entries {
        by_name.entry(e.name.clone()).or_default().push(e);
    }
    let blocked = blocked_by(&resolved);
    for (name, entries) in by_name {
        let mut copies: BTreeMap<(bool, String), Copy> = BTreeMap::new();
        for e in &entries {
            let item = e.owned.as_ref().map(|k| winner(&resolved, k));
            let key = match &e.owned {
                Some(k) => (false, k.to_string()),
                None => (true, e.real.display().to_string()),
            };
            let copy = copies.entry(key).or_insert_with(|| {
                let (description, hash) = read_skill(&e.real);
                let origin = match (&e.owned, &e.project) {
                    (Some(_), _) => None,
                    (None, Some(p)) => project_locks.get(p).and_then(|l| l.get(&name)).cloned(),
                    (None, None) => global_lock.get(&name).cloned(),
                };
                Copy {
                    path: e.real.display().to_string(),
                    scope: e.scope,
                    managed: e.owned.is_some(),
                    item: item.clone(),
                    adopted_into: adopted
                        .pending(&e.real)
                        .or_else(|| adopted.pending(&e.path))
                        .map(|a| a.source.clone()),
                    origin,
                    description,
                    content_hash: hash,
                    seen_by: Vec::new(),
                }
            });
            copy.scope = copy.scope.min(e.scope);
            copy.seen_by.push(Sighting {
                tool: e.tool.clone(),
                scope: e.scope,
                path: e.path.display().to_string(),
                link: e.link,
            });
        }
        let copies: Vec<Copy> = copies.into_values().collect();
        let mut s = skill(&name, copies, &blocked, &enabled, &resolved);
        s.handed_over = adopted
            .skills
            .iter()
            .any(|a| a.name == name && a.backup.is_some());
        out.skills.push(s);
    }
    Ok(out)
}

fn skill(
    name: &str,
    copies: Vec<Copy>,
    blocked: &BTreeMap<String, (String, Vec<String>)>,
    enabled: &BTreeSet<String>,
    resolved: &Resolved,
) -> Skill {
    let managed = copies.iter().filter(|c| c.managed).count();
    let others: Vec<&Copy> = copies.iter().filter(|c| !c.managed).collect();
    let state = match (managed, others.len()) {
        (_, 0) => State::Managed,
        (0, _) if others.iter().all(|c| c.adopted_into.is_some()) => State::Adopted,
        (0, _) => State::Unmanaged,
        _ => State::PartlyManaged,
    };
    let mut issues = Vec::new();
    if let Some((item, tools)) = blocked.get(name) {
        issues.push(Issue::Blocked {
            item: item.clone(),
            tools: tools.clone(),
        });
    }
    let mut per_tool: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &copies {
        for t in c
            .seen_by
            .iter()
            .map(|s| s.tool.as_str())
            .collect::<BTreeSet<_>>()
        {
            *per_tool.entry(t).or_default() += 1;
        }
    }
    let shadowed: Vec<String> = per_tool
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(t, _)| (*t).to_owned())
        .collect();
    if !shadowed.is_empty() {
        issues.push(Issue::Shadowed { tools: shadowed });
    }
    if copies.len() > 1 {
        let hashes: BTreeSet<&str> = copies.iter().map(|c| c.content_hash.as_str()).collect();
        issues.push(Issue::Copies {
            count: copies.len(),
            differ: hashes.len() > 1,
        });
    }
    if managed == 0 && others.iter().any(|c| c.scope == Scope::Global) {
        let missing: Vec<String> = enabled
            .iter()
            .filter(|t| !per_tool.contains_key(t.as_str()))
            .cloned()
            .collect();
        if !missing.is_empty() {
            issues.push(Issue::Missing { tools: missing });
        }
    }
    let key = ItemKey::new(ItemKind::Skill, name).ok();
    let item = key
        .as_ref()
        .and_then(|k| resolved.resolution.get(k))
        .filter(|r| r.enabled)
        .and_then(|r| r.winner.as_ref())
        .map(ToString::to_string);
    Skill {
        name: name.to_owned(),
        description: copies.iter().find_map(|c| c.description.clone()),
        state,
        item,
        handed_over: false,
        copies,
        issues,
    }
}

/// The winning item for `key`, as `source:kind/name` (or just the key).
fn winner(resolved: &Resolved, key: &ItemKey) -> String {
    resolved
        .resolution
        .get(key)
        .and_then(|r| r.winner.as_ref())
        .map_or_else(|| key.to_string(), ToString::to_string)
}

/// Skill name → (item, tools) where the last sync couldn't install it.
fn blocked_by(resolved: &Resolved) -> BTreeMap<String, (String, Vec<String>)> {
    let mut out: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    for (tool, placements) in &resolved.placements {
        for p in placements {
            if p.installed || p.id.key().kind() != ItemKind::Skill {
                continue;
            }
            out.entry(p.id.key().name().to_owned())
                .or_insert_with(|| (p.id.to_string(), Vec::new()))
                .1
                .push(tool.clone());
        }
    }
    out
}

fn read_skill(dir: &Path) -> (Option<String>, String) {
    let description = std::fs::read_to_string(dir.join(SKILL_FILE))
        .ok()
        .and_then(|t| Document::split(&t).ok()?.parse::<ItemMeta>().ok())
        .and_then(|m| m.description);
    let hash = crate::handover::folder_hash(dir).unwrap_or_default();
    (description, hash)
}

/// `npx skills`' record of global installs: `$XDG_STATE_HOME/skills/` when
/// that's set (and the home isn't a sandbox), else `~/.agents/`.
fn global_lock(ctx: &Ctx) -> BTreeMap<String, Origin> {
    let xdg = std::env::var_os("LOADOUT_HOME")
        .is_none()
        .then(|| std::env::var_os("XDG_STATE_HOME"))
        .flatten()
        .filter(|v| !v.is_empty())
        .map(|v| PathBuf::from(v).join("skills/.skill-lock.json"));
    let path = xdg.unwrap_or_else(|| ctx.paths.home.join(".agents/.skill-lock.json"));
    read_lock(&path)
}

#[derive(Deserialize)]
struct LockFile {
    #[serde(default)]
    skills: BTreeMap<String, LockEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LockEntry {
    source: String,
    #[serde(default)]
    source_type: String,
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    skill_path: Option<String>,
}

/// An `npx skills` lock file (`.skill-lock.json` or a project's
/// `skills-lock.json`): skill name → where it came from. Unreadable files
/// count as none.
pub fn read_lock(path: &Path) -> BTreeMap<String, Origin> {
    let Some(lock) = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<LockFile>(&t).ok())
    else {
        return BTreeMap::new();
    };
    lock.skills
        .into_iter()
        .map(|(name, e)| {
            let url = match (e.source_type.as_str(), e.source_url) {
                ("local", _) => None,
                (_, Some(u)) if !u.is_empty() => Some(u),
                ("github", None) => Some(format!("https://github.com/{}.git", e.source)),
                _ => None,
            };
            let origin = Origin {
                installer: "npx skills".to_owned(),
                source: e.source,
                url,
                path: e.skill_path,
            };
            (name, origin)
        })
        .collect()
}

/// The repo containing `dir` (the nearest folder with `.git`).
pub fn project_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|d| d.join(".git").exists())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_npx_skills_lock_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("lock.json");
        std::fs::write(
            &path,
            r#"{"version": 3, "skills": {
                "tdd": {"source": "acme/skills", "sourceType": "github", "sourceUrl": "https://github.com/acme/skills.git", "skillPath": "skills/tdd/SKILL.md"},
                "mine": {"source": "./x", "sourceType": "local"},
                "gh": {"source": "acme/other", "sourceType": "github"}
            }}"#,
        )
        .unwrap();
        let l = read_lock(&path);
        assert_eq!(
            l["tdd"].url.as_deref(),
            Some("https://github.com/acme/skills.git")
        );
        assert_eq!(l["tdd"].path.as_deref(), Some("skills/tdd/SKILL.md"));
        assert_eq!(l["mine"].url, None);
        assert_eq!(
            l["gh"].url.as_deref(),
            Some("https://github.com/acme/other.git")
        );
        assert!(read_lock(&tmp.path().join("missing.json")).is_empty());
    }
}

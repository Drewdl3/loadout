//! The data-driven target table.
//!
//! Built-in definitions live in the repo's `targets/*.toml` and are compiled
//! in. Files in the user's `targets/` config directory override a built-in
//! with the same `id` or add new targets.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use loadout_model::ItemKind;
use serde::Deserialize;

/// Built-in target definitions: (file name, contents).
pub const BUILTIN: &[(&str, &str)] = &[
    ("bob.toml", include_str!("../../../targets/bob.toml")),
    (
        "claude-code.toml",
        include_str!("../../../targets/claude-code.toml"),
    ),
    ("codex.toml", include_str!("../../../targets/codex.toml")),
    ("cursor.toml", include_str!("../../../targets/cursor.toml")),
    (
        "opencode.toml",
        include_str!("../../../targets/opencode.toml"),
    ),
    ("pi.toml", include_str!("../../../targets/pi.toml")),
];

#[derive(Debug, thiserror::Error)]
pub enum TableError {
    #[error("invalid target definition {file}: {message}")]
    Invalid { file: String, message: String },
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// One target (AI tool).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetDef {
    pub id: String,
    pub display: String,
    /// Paths whose presence means the tool is installed.
    #[serde(default)]
    pub detect: Vec<String>,
    #[serde(default)]
    pub skills: Option<DirSpec>,
    #[serde(default)]
    pub agents: Option<DirSpec>,
    #[serde(default)]
    pub mcp: Option<RendererSpec>,
    #[serde(default)]
    pub extras: BTreeMap<String, ExtraSpec>,
    #[serde(default)]
    pub plugins: Option<RendererSpec>,
}

impl TargetDef {
    /// Where items of `kind` go, if this target supports it via a directory.
    pub fn dir_for(&self, kind: ItemKind) -> Option<&DirSpec> {
        match kind {
            ItemKind::Skill => self.skills.as_ref(),
            ItemKind::Agent => self.agents.as_ref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirSpec {
    pub path: String,
    /// Project-relative path for project mode.
    #[serde(default)]
    pub project: Option<String>,
    pub layout: Layout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    /// `<path>/<name>/` mirrors `store/<kind>/<name>/`.
    DirPerItem,
    /// `<path>/<name>.md` mirrors the item's single file.
    FilePerItem,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererSpec {
    pub renderer: String,
    #[serde(default)]
    pub path: Option<String>,
    /// Project-relative config file for project mode.
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtraSpec {
    pub path: String,
    /// Project-relative path for project mode.
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default = "default_transform")]
    pub transform: String,
}

fn default_transform() -> String {
    "none".into()
}

/// All known targets, by id.
#[derive(Debug, Clone, Default)]
pub struct TargetTable {
    targets: BTreeMap<String, TargetDef>,
}

impl TargetTable {
    /// Parses `(file name, contents)` pairs; later entries override earlier
    /// ones with the same id.
    pub fn from_sources<'a>(
        sources: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, TableError> {
        let mut targets = BTreeMap::new();
        for (file, text) in sources {
            let def: TargetDef = toml::from_str(text).map_err(|e| TableError::Invalid {
                file: file.to_owned(),
                message: e.to_string(),
            })?;
            targets.insert(def.id.clone(), def);
        }
        Ok(TargetTable { targets })
    }

    /// Built-ins plus `*.toml` overrides from `user_dir` (if it exists).
    pub fn load(user_dir: &Path) -> Result<Self, TableError> {
        let mut user: Vec<(String, String)> = Vec::new();
        if user_dir.is_dir() {
            let io = |source| TableError::Io {
                path: user_dir.to_path_buf(),
                source,
            };
            for entry in std::fs::read_dir(user_dir).map_err(io)? {
                let path = entry.map_err(io)?.path();
                if path.extension().is_some_and(|e| e == "toml") {
                    let text = std::fs::read_to_string(&path).map_err(|source| TableError::Io {
                        path: path.clone(),
                        source,
                    })?;
                    user.push((path.display().to_string(), text));
                }
            }
            user.sort();
        }
        Self::from_sources(
            BUILTIN
                .iter()
                .copied()
                .chain(user.iter().map(|(f, t)| (f.as_str(), t.as_str()))),
        )
    }

    pub fn get(&self, id: &str) -> Option<&TargetDef> {
        self.targets.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &TargetDef> {
        self.targets.values()
    }
}

/// Expands a target path: `~` and `~/…` are relative to `home`; absolute
/// paths are used as is. Anything else is rejected (returns `None`).
pub fn expand_path(path: &str, home: &Path) -> Option<PathBuf> {
    if path == "~" {
        return Some(home.to_path_buf());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return Some(
            rest.split('/')
                .filter(|c| !c.is_empty())
                .fold(home.to_path_buf(), |p, c| p.join(c)),
        );
    }
    let p = Path::new(path);
    p.is_absolute().then(|| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_parse() {
        let t = TargetTable::from_sources(BUILTIN.iter().copied()).unwrap();
        let cc = t.get("claude-code").unwrap();
        assert_eq!(cc.display, "Claude Code");
        let skills = cc.dir_for(ItemKind::Skill).unwrap();
        assert_eq!(skills.path, "~/.claude/skills");
        assert_eq!(skills.layout, Layout::DirPerItem);
        assert_eq!(
            cc.dir_for(ItemKind::Agent).unwrap().layout,
            Layout::FilePerItem
        );
        assert_eq!(cc.extras["commands"].path, "~/.claude/commands");
    }

    #[test]
    fn all_v1_targets_are_built_in() {
        let t = TargetTable::from_sources(BUILTIN.iter().copied()).unwrap();
        let ids: Vec<&str> = t.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            ["bob", "claude-code", "codex", "cursor", "opencode", "pi"]
        );
        for d in t.iter() {
            let mcp = d.mcp.as_ref().expect("every v1 target renders MCP");
            crate::mcp::Format::from_renderer(&mcp.renderer).unwrap();
            assert!(d.skills.as_ref().unwrap().project.is_some(), "{}", d.id);
        }
        assert_eq!(
            t.get("pi").unwrap().extras["prompts"].path,
            "~/.pi/agent/prompts"
        );
        let bob = t.get("bob").unwrap();
        assert_eq!(bob.extras["rules"].path, "~/.bob/rules");
        assert_eq!(
            bob.mcp.as_ref().unwrap().path.as_deref(),
            Some("~/.bob/mcp_settings.json")
        );
    }

    #[test]
    fn user_files_override_and_extend() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("claude-code.toml"),
            "id = \"claude-code\"\ndisplay = \"Custom\"\n[skills]\npath = \"~/cc\"\nlayout = \"dir-per-item\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("acme.toml"),
            "id = \"acme-agent\"\ndisplay = \"Acme\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
        let t = TargetTable::load(dir.path()).unwrap();
        assert_eq!(t.get("claude-code").unwrap().display, "Custom");
        assert!(t.get("claude-code").unwrap().agents.is_none());
        assert!(t.get("acme-agent").is_some());
    }

    #[test]
    fn missing_user_dir_is_fine() {
        let t = TargetTable::load(Path::new("/definitely/not/here")).unwrap();
        assert!(t.get("claude-code").is_some());
    }

    #[test]
    fn invalid_definition_names_file() {
        let err = TargetTable::from_sources([("bad.toml", "id = 1")]).unwrap_err();
        assert!(err.to_string().contains("bad.toml"));
        let err = TargetTable::from_sources([("x.toml", "id=\"x\"\ndisplay=\"x\"\nbogus=1")])
            .unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }

    #[test]
    fn expands_paths() {
        let home = Path::new("/home/u");
        assert_eq!(expand_path("~", home).unwrap(), PathBuf::from("/home/u"));
        assert_eq!(
            expand_path("~/.claude/skills", home).unwrap(),
            home.join(".claude").join("skills")
        );
        assert_eq!(expand_path("relative/x", home), None);
        assert_eq!(expand_path("~other/x", home), None);
    }
}

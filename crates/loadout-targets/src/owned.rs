//! `owned.json`: which target entries Loadout created.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use loadout_core::links::Owned;
use serde::{Deserialize, Serialize};

use crate::fsutil::atomic_write;

pub const OWNED_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedFile {
    pub version: u32,
    /// Target id → entries.
    #[serde(default)]
    pub targets: BTreeMap<String, Vec<Owned>>,
    /// Target id → MCP servers Loadout manages in its config file.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mcp: BTreeMap<String, OwnedMcp>,
    /// Target id → settings keys Loadout manages for plugins.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, OwnedPlugins>,
}

/// Plugin registration in a target's settings file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedPlugins {
    pub settings: PathBuf,
    /// Names under `extraKnownMarketplaces`.
    pub marketplaces: BTreeSet<String>,
    /// Keys under `enabledPlugins`.
    pub enabled: BTreeSet<String>,
}

/// Managed MCP servers in one target config file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedMcp {
    /// The renderer that wrote the file (its format).
    pub renderer: String,
    /// The config file.
    pub path: PathBuf,
    /// The key (JSON object / TOML table) holding the servers.
    pub key: String,
    /// Server names Loadout wrote or adopted.
    pub servers: BTreeSet<String>,
    /// The subset whose entry contains resolved secret values
    /// (`allow_secrets_on_disk`); `lo doctor` lists them.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub secrets_on_disk: BTreeSet<String>,
}

impl Default for OwnedFile {
    fn default() -> Self {
        OwnedFile {
            version: OWNED_VERSION,
            targets: BTreeMap::new(),
            mcp: BTreeMap::new(),
            plugins: BTreeMap::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OwnedError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is corrupt ({source}); move it aside and re-run `lo sync`")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("{path} has unsupported version {version}")]
    Version { path: String, version: u32 },
}

impl OwnedFile {
    /// Loads `path`; a missing file is an empty record.
    pub fn load(path: &Path) -> Result<Self, OwnedError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(OwnedError::Io {
                    path: path.display().to_string(),
                    source,
                });
            }
        };
        let file: OwnedFile = serde_json::from_str(&text).map_err(|source| OwnedError::Parse {
            path: path.display().to_string(),
            source,
        })?;
        if file.version != OWNED_VERSION {
            return Err(OwnedError::Version {
                path: path.display().to_string(),
                version: file.version,
            });
        }
        Ok(file)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        json.push(b'\n');
        atomic_write(path, &json, false)
    }

    pub fn entries(&self, target: &str) -> &[Owned] {
        self.targets.get(target).map_or(&[], Vec::as_slice)
    }

    pub fn set_mcp(&mut self, target: &str, owned: OwnedMcp) {
        if owned.servers.is_empty() {
            self.mcp.remove(target);
        } else {
            self.mcp.insert(target.to_owned(), owned);
        }
    }

    pub fn set(&mut self, target: &str, entries: Vec<Owned>) {
        if entries.is_empty() {
            self.targets.remove(target);
        } else {
            self.targets.insert(target.to_owned(), entries);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loadout_model::LinkMode;

    #[test]
    fn round_trips_and_missing_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owned.json");
        assert_eq!(OwnedFile::load(&path).unwrap(), OwnedFile::default());

        let mut f = OwnedFile::default();
        f.set(
            "claude-code",
            vec![Owned {
                item: "skill/a".parse().unwrap(),
                dest: "/h/.claude/skills/a".into(),
                mode: LinkMode::Symlink,
                content_hash: "blake3:00".into(),
            }],
        );
        f.save(&path).unwrap();
        let loaded = OwnedFile::load(&path).unwrap();
        assert_eq!(loaded, f);
        assert_eq!(loaded.entries("claude-code").len(), 1);
        assert!(loaded.entries("pi").is_empty());
    }

    #[test]
    fn corrupt_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owned.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(matches!(
            OwnedFile::load(&path),
            Err(OwnedError::Parse { .. })
        ));
    }
}

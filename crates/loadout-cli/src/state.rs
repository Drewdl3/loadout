//! `resolved.json`: the resolved item set from the last successful sync.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use loadout_core::resolve::Resolution;
use loadout_model::{ItemId, ItemKind, LinkMode};
use loadout_targets::fsutil::atomic_write;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const RESOLVED_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resolved {
    pub version: u32,
    pub sources: Vec<ResolvedSource>,
    /// Winners of the target-independent resolution (for `list`).
    pub items: Vec<ResolvedItem>,
    /// The full target-independent resolution with explanation trails.
    pub resolution: Resolution,
    /// Target id → the entries it should contain.
    pub placements: BTreeMap<String, Vec<Placement>>,
}

/// An enabled winner's entry in one target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    /// The winner for this target (may differ from the overall winner when
    /// items restrict their `targets`).
    pub id: ItemId,
    pub dest: PathBuf,
    pub mode: LinkMode,
    /// False when something Loadout doesn't own occupies `dest`, or
    /// placing it failed.
    pub installed: bool,
}

impl Default for Resolved {
    fn default() -> Self {
        Resolved {
            version: RESOLVED_VERSION,
            sources: Vec::new(),
            items: Vec::new(),
            resolution: Resolution::default(),
            placements: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResolvedSource {
    /// Source id from `LOADOUT.md`.
    pub name: String,
    pub url: String,
    pub commit: String,
    /// The source whose `upstream:` pulled this one in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// URLs this source's `LOADOUT.md` names as `upstream:`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upstream: Vec<String>,
    /// The source's default layer (its subscription or company config mapping,
    /// else its `LOADOUT.md`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ResolvedItem {
    /// `source:kind/name`.
    #[schemars(with = "String")]
    pub id: ItemId,
    #[schemars(with = "String")]
    pub kind: ItemKind,
    pub name: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub enabled: bool,
    /// `blake3:<hex>` over the item's files.
    pub content_hash: String,
    /// Optional include-list of target ids from the item's `loadout:` block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub co_authors: Vec<String>,
}

impl Resolved {
    /// Loads `path`. A missing file, or one written by an older version,
    /// means nothing usable has been synced yet (it is a cache that every
    /// sync rewrites).
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        #[derive(Deserialize)]
        struct Version {
            version: u32,
        }
        let v: Version = serde_json::from_str(&text)
            .with_context(|| format!("{} is corrupt; re-run `lo sync`", path.display()))?;
        if v.version < RESOLVED_VERSION {
            return Ok(None);
        }
        if v.version > RESOLVED_VERSION {
            bail!(
                "{} was written by a newer Loadout (version {})",
                path.display(),
                v.version
            );
        }
        let r: Resolved = serde_json::from_str(&text)
            .with_context(|| format!("{} is corrupt; re-run `lo sync`", path.display()))?;
        Ok(Some(r))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut json = serde_json::to_vec_pretty(self)?;
        json.push(b'\n');
        atomic_write(path, &json, false).with_context(|| format!("writing {}", path.display()))
    }
}

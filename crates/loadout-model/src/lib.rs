//! Loadout data model: items, sources, company config, profile, layers, ids.
//!
//! Pure types with serde support. No I/O.

pub mod company_config;
pub mod config;
pub mod duration;
pub mod frontmatter;
pub mod id;
pub mod item;
pub mod layer;
pub mod lock;
pub mod manifest;
pub mod mcp;
pub mod secret;
pub mod template;
pub mod time;

pub use company_config::{CompanyConfig, GroupDef, Policy, Rule, Signer};
pub use config::{Config, ConfigDoc, Groups, LinkMode, Membership, SourceSub, TargetsConfig};
pub use id::{ItemId, ItemKey, ItemKind};
pub use item::{ItemMeta, LoadoutBlock, Mode};
pub use layer::{Layer, LayerModel, Profile};
pub use lock::{Lock, LockedItem, LockedSource};
pub use manifest::{Manifest, ManifestDoc, SourcePaths, Upstream};
pub use mcp::{McpFrontmatter, McpServer, Transport};
pub use secret::{SecretRef, Template};

/// Errors produced while parsing or validating model types.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("invalid item kind {0:?} (expected skill, mcp, agent, plugin or extra)")]
    InvalidKind(String),
    #[error(
        "invalid item name {0:?} (use lowercase letters, digits, '-', '_' or '.', starting with a letter or digit)"
    )]
    InvalidItemName(String),
    #[error("invalid source name {0:?} (use kebab-case: lowercase letters, digits and single '-')")]
    InvalidSourceName(String),
    #[error("invalid item key {0:?} (expected kind/name)")]
    InvalidItemKey(String),
    #[error("invalid item id {0:?} (expected source:kind/name)")]
    InvalidItemId(String),
    #[error("frontmatter starts with '---' but has no closing '---' line")]
    UnterminatedFrontmatter,
    #[error("invalid YAML frontmatter: {0}")]
    Yaml(String),
    #[error("unsupported LOADOUT.md schema version {0} (this build supports loadout: 1)")]
    UnsupportedManifestVersion(u32),
    #[error("invalid LOADOUT.md: {0}")]
    InvalidManifest(String),
    #[error("invalid config.toml: {0}")]
    Config(String),
    #[error("invalid company config: {0}")]
    CompanyConfig(String),
    #[error("invalid secret reference {0:?}: {1}")]
    InvalidSecretRef(String, String),
    #[error("invalid MCP server: {0}")]
    InvalidMcp(String),
    #[error("invalid loadout.lock: {0}")]
    Lock(String),
    #[error("invalid timestamp {0:?} (expected YYYY-MM-DDTHH:MM:SSZ)")]
    Timestamp(String),
    #[error("invalid duration {0:?} (use e.g. 30m, 1h, 24h, 7d)")]
    Duration(String),
}

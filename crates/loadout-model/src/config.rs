//! Local configuration: `~/.config/loadout/config.toml`.
//!
//! [`Config`] is the typed, read-only view. [`ConfigDoc`] edits the file
//! while preserving the user's comments and formatting.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use crate::ModelError;

/// Default `sync_interval`.
pub const DEFAULT_SYNC_INTERVAL: &str = "1h";

/// Default `profile_refresh_interval`.
pub const DEFAULT_PROFILE_REFRESH_INTERVAL: &str = "24h";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// URL of the company config repo, once `lo init` has run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_interval: Option<String>,
    /// How often scheduled syncs re-run membership discovery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_refresh_interval: Option<String>,
    /// Cached group membership: layer → groups.
    pub profile: BTreeMap<String, Groups>,
    /// Manual subscriptions.
    #[serde(rename = "source")]
    pub sources: Vec<SourceSub>,
    /// User overrides: item id → enabled.
    pub toggles: BTreeMap<String, bool>,
    /// Manual membership choices (`lo join` / `leave` / `profile set`).
    #[serde(skip_serializing_if = "Membership::is_empty")]
    pub membership: Membership,
    /// Equal-rank conflict choices (`lo prefer`): `kind/name` → source.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub prefer: BTreeMap<String, String>,
    pub targets: TargetsConfig,
    /// Secret handling.
    #[serde(skip_serializing_if = "SecretsConfig::is_default")]
    pub secrets: SecretsConfig,
    /// Local review policy.
    #[serde(skip_serializing_if = "PolicyConfig::is_default")]
    pub policy: PolicyConfig,
}

/// `[policy]`: local additions to the company config's review policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PolicyConfig {
    /// Sources (`LOADOUT.md` name or URL) whose updates apply without
    /// review, in addition to the company config's `auto_apply` layers.
    pub auto_apply: Vec<String>,
}

impl PolicyConfig {
    pub fn is_default(&self) -> bool {
        self.auto_apply.is_empty()
    }
}

/// `[secrets]`: local secret settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SecretsConfig {
    /// Overrides the company config's resolver chain for `secret://` references.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain: Option<Vec<String>>,
    /// Allow writing resolved secrets into target configs when no safer
    /// delivery exists.
    pub allow_secrets_on_disk: bool,
}

impl SecretsConfig {
    pub fn is_default(&self) -> bool {
        self == &SecretsConfig::default()
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        let c: Config = toml::from_str(text).map_err(|e| ModelError::Config(e.to_string()))?;
        for (key, v) in [
            ("sync_interval", &c.sync_interval),
            ("profile_refresh_interval", &c.profile_refresh_interval),
        ] {
            if let Some(v) = v {
                crate::duration::parse_duration(v)
                    .map_err(|e| ModelError::Config(format!("{key}: {e}")))?;
            }
        }
        Ok(c)
    }

    pub fn profile_refresh_interval(&self) -> Result<std::time::Duration, ModelError> {
        crate::duration::parse_duration(
            self.profile_refresh_interval
                .as_deref()
                .unwrap_or(DEFAULT_PROFILE_REFRESH_INTERVAL),
        )
    }

    pub fn sync_interval(&self) -> &str {
        self.sync_interval
            .as_deref()
            .unwrap_or(DEFAULT_SYNC_INTERVAL)
    }

    /// The manual subscription for `url`, compared after normalization.
    pub fn source(&self, url: &str) -> Option<&SourceSub> {
        let url = normalize_url(url);
        self.sources.iter().find(|s| normalize_url(&s.url) == url)
    }
}

impl schemars::JsonSchema for Groups {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Groups".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "anyOf": [
                { "type": "string" },
                { "type": "array", "items": { "type": "string" } }
            ]
        })
    }
}

/// One or more group names. Deserializes from a string or an array, so
/// `company = "acme"` and `team = ["payments-dev"]` both work.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Groups(pub Vec<String>);

impl<'de> Deserialize<'de> for Groups {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum OneOrMany {
            One(String),
            Many(Vec<String>),
        }
        Ok(match OneOrMany::deserialize(d)? {
            OneOrMany::One(s) => Groups(vec![s]),
            OneOrMany::Many(v) => Groups(v),
        })
    }
}

/// Groups the user joined or left by hand, as `layer:group`. Discovery
/// results are combined with these on every profile refresh.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Membership {
    pub joined: Vec<String>,
    pub left: Vec<String>,
}

impl Membership {
    pub fn is_empty(&self) -> bool {
        self.joined.is_empty() && self.left.is_empty()
    }

    pub fn has_joined(&self, group_id: &str) -> bool {
        self.joined.iter().any(|u| u == group_id)
    }

    pub fn has_left(&self, group_id: &str) -> bool {
        self.left.iter().any(|u| u == group_id)
    }
}

/// A manual source subscription (`[[source]]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceSub {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Tie-breaker among equal-rank sources; higher wins.
    #[serde(default)]
    pub priority: i64,
    /// Optional branch, tag or commit to track instead of the default branch.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
}

impl SourceSub {
    pub fn new(url: impl Into<String>) -> Self {
        SourceSub {
            url: url.into(),
            layer: None,
            group: None,
            priority: 0,
            git_ref: None,
        }
    }
}

/// How items are placed into a target directory.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum LinkMode {
    /// Symlink (Unix) or NTFS junction (Windows) into the store.
    #[default]
    Symlink,
    /// Copy files out of the store.
    Copy,
}

impl LinkMode {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkMode::Symlink => "symlink",
            LinkMode::Copy => "copy",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TargetsConfig {
    /// Enabled target ids. `None` means "not chosen yet": targets whose
    /// `detect` paths exist are used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<Vec<String>>,
    pub link_mode: LinkMode,
    /// Per-target link mode overrides.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub modes: BTreeMap<String, LinkMode>,
}

impl TargetsConfig {
    pub fn link_mode_for(&self, target: &str) -> LinkMode {
        self.modes.get(target).copied().unwrap_or(self.link_mode)
    }
}

/// Normalizes a source URL for comparison and hashing: trims whitespace,
/// trailing slashes, and a trailing `.git`.
pub fn normalize_url(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    u.strip_suffix(".git").unwrap_or(u).to_owned()
}

/// A format-preserving editor for `config.toml`.
#[derive(Debug, Clone, Default)]
pub struct ConfigDoc {
    doc: DocumentMut,
}

impl ConfigDoc {
    /// Parses and validates `text`.
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        Config::parse(text)?;
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|e| ModelError::Config(e.to_string()))?;
        Ok(ConfigDoc { doc })
    }

    /// The typed view of the current document.
    pub fn config(&self) -> Config {
        Config::parse(&self.doc.to_string()).expect("ConfigDoc is always valid")
    }

    /// Adds a `[[source]]`. Returns `false` (and changes nothing) if a
    /// source with the same normalized URL already exists.
    pub fn add_source(&mut self, sub: &SourceSub) -> bool {
        if self.config().source(&sub.url).is_some() {
            return false;
        }
        let mut t = Table::new();
        t.insert("url", value(&sub.url));
        if let Some(layer) = &sub.layer {
            t.insert("layer", value(layer));
        }
        if let Some(group) = &sub.group {
            t.insert("group", value(group));
        }
        if sub.priority != 0 {
            t.insert("priority", value(sub.priority));
        }
        if let Some(r) = &sub.git_ref {
            t.insert("ref", value(r));
        }
        match self.doc.get_mut("source") {
            Some(Item::ArrayOfTables(a)) => a.push(t),
            _ => {
                let mut a = ArrayOfTables::new();
                a.push(t);
                self.doc.insert("source", Item::ArrayOfTables(a));
            }
        }
        true
    }

    /// Sets `[toggles] "<id>" = on`.
    pub fn set_toggle(&mut self, id: &str, on: bool) {
        self.table("toggles").insert(id, value(on));
    }

    /// Sets `[prefer] "<kind/name>" = "<source>"`.
    pub fn set_prefer(&mut self, key: &str, source: &str) {
        self.table("prefer").insert(key, value(source));
    }

    pub fn set_company_config(&mut self, url: &str) {
        self.doc.insert("company_config", value(url));
    }

    /// Replaces `[profile]`. The `company` layer is written as a string,
    /// others as arrays.
    pub fn set_profile(&mut self, profile: &BTreeMap<String, Vec<String>>) {
        let t = self.table("profile");
        t.clear();
        for (layer, groups) in profile {
            if groups.is_empty() {
                continue;
            }
            if layer == "company" && groups.len() == 1 {
                t.insert(layer, value(&groups[0]));
            } else {
                t.insert(layer, value(string_array(groups)));
            }
        }
    }

    /// Replaces `[membership]`, removing it when empty.
    pub fn set_membership(&mut self, m: &Membership) {
        if m.is_empty() {
            self.doc.remove("membership");
            return;
        }
        let t = self.table("membership");
        t.clear();
        t.insert("joined", value(string_array(&m.joined)));
        t.insert("left", value(string_array(&m.left)));
    }

    /// Sets `sync_interval`.
    pub fn set_sync_interval(&mut self, interval: &str) {
        self.doc.insert("sync_interval", value(interval));
    }

    /// Removes a top-level table (e.g. `toggles`, `prefer`) entirely.
    pub fn remove_table(&mut self, name: &str) {
        self.doc.remove(name);
    }

    /// Sets `[targets] enabled`.
    pub fn set_targets_enabled(&mut self, ids: &[String]) {
        self.table("targets")
            .insert("enabled", value(string_array(ids)));
    }

    /// Sets `[targets.modes] <target> = "<mode>"`.
    pub fn set_target_mode(&mut self, target: &str, mode: LinkMode) {
        let targets = self.table("targets");
        if !targets.contains_table("modes") {
            let mut t = Table::new();
            t.set_implicit(false);
            targets.insert("modes", Item::Table(t));
        }
        targets["modes"]
            .as_table_mut()
            .expect("checked above")
            .insert(target, value(mode.as_str()));
    }

    /// The top-level table `name`, created if missing.
    fn table(&mut self, name: &str) -> &mut Table {
        if !self.doc.contains_table(name) {
            self.doc.insert(name, Item::Table(Table::new()));
        }
        self.doc[name].as_table_mut().expect("checked above")
    }

    /// Removes the `[[source]]` with the given normalized URL. Returns
    /// whether one was removed.
    pub fn remove_source(&mut self, url: &str) -> bool {
        let url = normalize_url(url);
        let Some(Item::ArrayOfTables(a)) = self.doc.get_mut("source") else {
            return false;
        };
        let before = a.len();
        a.retain(|t| {
            t.get("url")
                .and_then(Item::as_str)
                .is_none_or(|u| normalize_url(u) != url)
        });
        let removed = a.len() != before;
        if a.is_empty() {
            self.doc.remove("source");
        }
        removed
    }
}

fn string_array(items: &[String]) -> toml_edit::Array {
    items.iter().map(String::as_str).collect()
}

impl std::fmt::Display for ConfigDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"company_config = "https://github.com/acme/agent-config"
sync_interval = "1h"

[profile]                       # resolved membership, cached
company = "acme"
org = ["eng"]
team = ["payments-dev"]
product = ["billing"]
role = ["developer"]

[[source]]                      # manual subscriptions
url = "https://github.com/someone/extra-skills"
layer = "role"
group = "developer"
priority = 0

[toggles]
"payments-skills:skill/write-spec" = false
"eng-skills:mcp/sentry" = true

[targets]
enabled = ["claude-code", "pi"]
link_mode = "symlink"
"#;

    #[test]
    fn parses_secrets_section() {
        let c = Config::parse(
            "[secrets]\nchain = [\"env\", \"op://Acme/{name}\"]\nallow_secrets_on_disk = true\n",
        )
        .unwrap();
        assert_eq!(c.secrets.chain.as_ref().unwrap().len(), 2);
        assert!(c.secrets.allow_secrets_on_disk);
        assert!(Config::parse("").unwrap().secrets.is_default());
        assert!(Config::parse("[secrets]\nbogus = 1\n").is_err());
    }

    #[test]
    fn parses_example() {
        let c = Config::parse(EXAMPLE).unwrap();
        assert_eq!(
            c.company_config.as_deref(),
            Some("https://github.com/acme/agent-config")
        );
        assert_eq!(c.profile["company"], Groups(vec!["acme".into()]));
        assert_eq!(c.profile["team"], Groups(vec!["payments-dev".into()]));
        assert_eq!(c.sources.len(), 1);
        assert_eq!(c.sources[0].group.as_deref(), Some("developer"));
        assert!(!c.toggles["payments-skills:skill/write-spec"]);
        assert_eq!(
            c.targets.enabled.as_deref(),
            Some(&["claude-code".to_owned(), "pi".to_owned()][..])
        );
        assert_eq!(c.targets.link_mode_for("pi"), LinkMode::Symlink);
    }

    #[test]
    fn empty_config_has_defaults() {
        let c = Config::parse("").unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.sync_interval(), "1h");
        assert_eq!(c.targets.enabled, None);
    }

    #[test]
    fn validates_intervals() {
        assert!(Config::parse("sync_interval = \"2h\"").is_ok());
        assert!(Config::parse("sync_interval = \"soon\"").is_err());
        let c = Config::parse("profile_refresh_interval = \"12h\"").unwrap();
        assert_eq!(c.profile_refresh_interval().unwrap().as_secs(), 43_200);
        assert_eq!(
            Config::default()
                .profile_refresh_interval()
                .unwrap()
                .as_secs(),
            86_400
        );
    }

    #[test]
    fn rejects_unknown_top_level_keys() {
        assert!(Config::parse("company_configg = \"x\"").is_err());
    }

    #[test]
    fn per_target_link_mode() {
        let c = Config::parse("[targets.modes]\nclaude-code = \"copy\"\n").unwrap();
        assert_eq!(c.targets.link_mode_for("claude-code"), LinkMode::Copy);
        assert_eq!(c.targets.link_mode_for("pi"), LinkMode::Symlink);
    }

    #[test]
    fn add_source_preserves_comments_and_dedupes() {
        let mut doc = ConfigDoc::parse(EXAMPLE).unwrap();
        let mut sub = SourceSub::new("https://example.com/acme/team-skills.git");
        sub.priority = 5;
        assert!(doc.add_source(&sub));
        assert!(!doc.add_source(&SourceSub::new("https://example.com/acme/team-skills/")));
        let text = doc.to_string();
        assert!(text.contains("# resolved membership, cached"));
        let c = Config::parse(&text).unwrap();
        assert_eq!(c.sources.len(), 2);
        assert_eq!(c.sources[1].priority, 5);
    }

    #[test]
    fn add_source_to_empty_config() {
        let mut doc = ConfigDoc::parse("").unwrap();
        assert!(doc.add_source(&SourceSub::new("/tmp/repo")));
        assert_eq!(doc.to_string(), "[[source]]\nurl = \"/tmp/repo\"\n");
    }

    #[test]
    fn remove_source() {
        let mut doc = ConfigDoc::parse(EXAMPLE).unwrap();
        assert!(!doc.remove_source("https://example.com/nope"));
        assert!(doc.remove_source("https://github.com/someone/extra-skills.git"));
        let c = doc.config();
        assert!(c.sources.is_empty());
        assert!(!doc.to_string().contains("[[source]]"));
    }

    #[test]
    fn toggles_and_prefer_round_trip_and_keep_comments() {
        let mut doc = ConfigDoc::parse(EXAMPLE).unwrap();
        doc.set_toggle("payments-skills:skill/write-spec", true);
        doc.set_toggle("new-src:skill/x", false);
        doc.set_prefer("skill/x", "new-src");
        let text = doc.to_string();
        assert!(text.contains("# resolved membership, cached"));
        let c = Config::parse(&text).unwrap();
        assert!(c.toggles["payments-skills:skill/write-spec"]);
        assert!(!c.toggles["new-src:skill/x"]);
        assert_eq!(c.prefer["skill/x"], "new-src");

        let mut empty = ConfigDoc::parse("").unwrap();
        empty.set_prefer("skill/x", "a");
        assert_eq!(empty.to_string(), "[prefer]\n\"skill/x\" = \"a\"\n");
    }

    #[test]
    fn profile_membership_company_config_and_targets_edits() {
        let mut doc = ConfigDoc::parse("# keep me\n").unwrap();
        doc.set_company_config("https://example.com/acme/company-config");
        doc.set_profile(
            &[
                ("company".to_owned(), vec!["acme".to_owned()]),
                ("team".to_owned(), vec!["a".to_owned(), "b".to_owned()]),
                ("role".to_owned(), vec![]),
            ]
            .into(),
        );
        doc.set_membership(&Membership {
            joined: vec!["product:billing".into()],
            left: vec!["team:b".into()],
        });
        doc.set_targets_enabled(&["claude-code".to_owned()]);
        let text = doc.to_string();
        assert!(text.contains("# keep me"), "{text}");
        let c = Config::parse(&text).unwrap();
        assert_eq!(
            c.company_config.as_deref(),
            Some("https://example.com/acme/company-config")
        );
        assert_eq!(c.profile["company"], Groups(vec!["acme".into()]));
        assert_eq!(c.profile["team"], Groups(vec!["a".into(), "b".into()]));
        assert!(!c.profile.contains_key("role"));
        assert!(c.membership.has_joined("product:billing"));
        assert!(c.membership.has_left("team:b"));
        assert_eq!(
            c.targets.enabled.as_deref(),
            Some(&["claude-code".to_owned()][..])
        );
        assert!(text.contains("company = \"acme\""), "{text}");

        doc.set_membership(&Membership::default());
        assert!(!doc.to_string().contains("[membership]"));
    }

    #[test]
    fn normalizes_urls() {
        assert_eq!(normalize_url(" https://x/y.git/ "), "https://x/y");
        assert_eq!(normalize_url("/tmp/repo"), "/tmp/repo");
    }
}

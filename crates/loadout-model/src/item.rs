//! Item metadata: the frontmatter of `SKILL.md`, agent and extra files,
//! including Loadout's optional `loadout:` block.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How an item's enabled state is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Always enabled; user toggles are ignored.
    Required,
    /// Enabled unless the user toggles it off.
    DefaultOn,
    /// Disabled unless the user toggles it on.
    DefaultOff,
}

/// The `loadout:` block in item frontmatter, also used for `LOADOUT.md`
/// `defaults:`. Every field is optional; unset fields fall back to the
/// source defaults, then to built-in defaults.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct LoadoutBlock {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Axis → allowed groups. All listed axes must match (AND); values within
    /// an axis are alternatives (OR).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub applies_to: BTreeMap<String, Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub product: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overridable: Option<bool>,
    /// Optional include-list of target ids.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<String>>,
    /// Who wrote the item, e.g. `Ada Lovelace` or `Ada Lovelace <ada@example.com>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Other people who contributed to it. A single string is accepted
    /// too; `coAuthors` / `coAuthor` are accepted as spellings.
    #[serde(
        alias = "coAuthors",
        alias = "coAuthor",
        alias = "co_author",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub co_authors: Vec<String>,
    /// Unknown keys, preserved.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl LoadoutBlock {
    /// Returns `self` with unset fields filled from `defaults`.
    pub fn or_defaults(&self, defaults: &LoadoutBlock) -> LoadoutBlock {
        LoadoutBlock {
            layer: self.layer.clone().or_else(|| defaults.layer.clone()),
            group: self.group.clone().or_else(|| defaults.group.clone()),
            applies_to: if self.applies_to.is_empty() {
                defaults.applies_to.clone()
            } else {
                self.applies_to.clone()
            },
            tags: if self.tags.is_empty() {
                defaults.tags.clone()
            } else {
                self.tags.clone()
            },
            product: if self.product.is_empty() {
                defaults.product.clone()
            } else {
                self.product.clone()
            },
            mode: self.mode.or(defaults.mode),
            locked: self.locked.or(defaults.locked),
            overridable: self.overridable.or(defaults.overridable),
            targets: self.targets.clone().or_else(|| defaults.targets.clone()),
            author: self.author.clone().or_else(|| defaults.author.clone()),
            co_authors: if self.co_authors.is_empty() {
                defaults.co_authors.clone()
            } else {
                self.co_authors.clone()
            },
            extra: self.extra.clone(),
        }
    }
}

/// A string or a list of strings.
fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

/// Frontmatter common to Markdown items (skills, subagents, extras).
/// Loadout only reads these fields; files are distributed verbatim, so
/// unknown keys are never rewritten.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loadout: Option<LoadoutBlock>,
    /// Unknown keys (other tools' fields), preserved.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::Document;

    const SKILL: &str = r#"---
name: write-spec
description: Generate a feature spec in the Payments house format.
allowed-tools: [Read, Write]
loadout:
  layer: squad
  group: checkout
  applies_to:
    role: [developer, product-manager]
  tags: [specs, writing]
  product: [billing]
  mode: default-on
  locked: false
  overridable: true
  targets: [claude-code, pi]
  future-key: 42
---
Body.
"#;

    #[test]
    fn parses_example_and_preserves_unknown_keys() {
        let meta: ItemMeta = Document::split(SKILL).unwrap().parse().unwrap();
        assert_eq!(meta.name.as_deref(), Some("write-spec"));
        assert_eq!(
            meta.extra.get("allowed-tools"),
            Some(&serde_json::json!(["Read", "Write"]))
        );
        let g = meta.loadout.unwrap();
        assert_eq!(g.layer.as_deref(), Some("squad"));
        assert_eq!(g.group.as_deref(), Some("checkout"));
        assert_eq!(
            g.applies_to["role"],
            vec!["developer".to_owned(), "product-manager".to_owned()]
        );
        assert_eq!(g.mode, Some(Mode::DefaultOn));
        assert_eq!(g.locked, Some(false));
        assert_eq!(g.overridable, Some(true));
        assert_eq!(
            g.targets,
            Some(vec!["claude-code".to_owned(), "pi".to_owned()])
        );
        assert_eq!(g.extra.get("future-key"), Some(&serde_json::json!(42)));
    }

    #[test]
    fn plain_skill_without_loadout_block() {
        let meta: ItemMeta = Document::split("---\ndescription: hi\n---\n")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(meta.description.as_deref(), Some("hi"));
        assert!(meta.loadout.is_none());
    }

    #[test]
    fn author_and_co_authors() {
        let parse = |fm: &str| -> LoadoutBlock {
            let text = format!("---\n{fm}\n---\n");
            let meta: ItemMeta = Document::split(&text).unwrap().parse().unwrap();
            meta.loadout.unwrap()
        };
        let g = parse("loadout:\n  author: Ada\n  co_authors: [Grace, Linus]");
        assert_eq!(g.author.as_deref(), Some("Ada"));
        assert_eq!(g.co_authors, ["Grace", "Linus"]);
        assert_eq!(
            parse("loadout: { coAuthors: [Grace] }").co_authors,
            ["Grace"]
        );
        assert_eq!(parse("loadout: { coAuthor: Grace }").co_authors, ["Grace"]);
        assert!(parse("loadout: { coAuthor: Grace }").extra.is_empty());
        let json =
            serde_json::to_value(parse("loadout: { author: Ada, co_authors: Grace }")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"author": "Ada", "co_authors": ["Grace"]})
        );
        // Source defaults fill them in.
        let defaults = LoadoutBlock {
            author: Some("Team".into()),
            ..Default::default()
        };
        assert_eq!(
            LoadoutBlock::default()
                .or_defaults(&defaults)
                .author
                .as_deref(),
            Some("Team")
        );
    }

    #[test]
    fn rejects_unknown_mode() {
        let doc = Document::split("---\nloadout: { mode: sometimes }\n---\n").unwrap();
        assert!(doc.parse::<ItemMeta>().is_err());
    }

    #[test]
    fn defaults_fill_unset_fields_only() {
        let item = LoadoutBlock {
            layer: Some("squad".into()),
            ..Default::default()
        };
        let defaults = LoadoutBlock {
            layer: Some("team".into()),
            group: Some("payments-dev".into()),
            mode: Some(Mode::Required),
            ..Default::default()
        };
        let merged = item.or_defaults(&defaults);
        assert_eq!(merged.layer.as_deref(), Some("squad"));
        assert_eq!(merged.group.as_deref(), Some("payments-dev"));
        assert_eq!(merged.mode, Some(Mode::Required));
    }
}

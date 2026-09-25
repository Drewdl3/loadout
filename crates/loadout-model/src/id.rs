//! Identifiers: item kinds, names, and fully-qualified item ids
//! (`source:kind/name`).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ModelError;

/// The kind of a distributable item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemKind {
    Skill,
    Mcp,
    Agent,
    Plugin,
    Extra,
}

impl ItemKind {
    pub const ALL: [ItemKind; 5] = [
        ItemKind::Skill,
        ItemKind::Mcp,
        ItemKind::Agent,
        ItemKind::Plugin,
        ItemKind::Extra,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ItemKind::Skill => "skill",
            ItemKind::Mcp => "mcp",
            ItemKind::Agent => "agent",
            ItemKind::Plugin => "plugin",
            ItemKind::Extra => "extra",
        }
    }
}

impl fmt::Display for ItemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ItemKind {
    type Err = ModelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ItemKind::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| ModelError::InvalidKind(s.to_owned()))
    }
}

/// Validates an item name. Names become file and directory names in the
/// store and in target directories, so they are restricted to lowercase
/// ASCII letters, digits, `-`, `_` and `.`, must start with a letter or
/// digit, and are at most 128 bytes.
pub fn validate_item_name(name: &str) -> Result<(), ModelError> {
    let valid = !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && name.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
        })
        && !name.contains("..");
    if valid {
        Ok(())
    } else {
        Err(ModelError::InvalidItemName(name.to_owned()))
    }
}

/// Validates a source id: kebab-case (`payments-skills`).
pub fn validate_source_name(name: &str) -> Result<(), ModelError> {
    let valid = !name.is_empty()
        && name.len() <= 128
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    if valid {
        Ok(())
    } else {
        Err(ModelError::InvalidSourceName(name.to_owned()))
    }
}

/// An item identity without its source: `kind/name`. Items from different
/// sources with the same key compete during resolution.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemKey {
    kind: ItemKind,
    name: String,
}

impl ItemKey {
    pub fn new(kind: ItemKind, name: impl Into<String>) -> Result<Self, ModelError> {
        let name = name.into();
        validate_item_name(&name)?;
        Ok(ItemKey { kind, name })
    }

    pub fn kind(&self) -> ItemKind {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for ItemKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.kind, self.name)
    }
}

impl FromStr for ItemKey {
    type Err = ModelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind, name) = s
            .split_once('/')
            .ok_or_else(|| ModelError::InvalidItemKey(s.to_owned()))?;
        ItemKey::new(kind.parse()?, name)
    }
}

/// A fully-qualified item id: `source:kind/name`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemId {
    source: String,
    key: ItemKey,
}

impl ItemId {
    pub fn new(source: impl Into<String>, key: ItemKey) -> Result<Self, ModelError> {
        let source = source.into();
        validate_source_name(&source)?;
        Ok(ItemId { source, key })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn key(&self) -> &ItemKey {
        &self.key
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.source, self.key)
    }
}

impl FromStr for ItemId {
    type Err = ModelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (source, key) = s
            .split_once(':')
            .ok_or_else(|| ModelError::InvalidItemId(s.to_owned()))?;
        ItemId::new(source, key.parse()?)
    }
}

macro_rules! string_serde {
    ($ty:ty) => {
        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

string_serde!(ItemKind);
string_serde!(ItemKey);
string_serde!(ItemId);

macro_rules! string_schema {
    ($ty:ty, $name:literal, $desc:literal) => {
        impl schemars::JsonSchema for $ty {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                $name.into()
            }
            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({ "type": "string", "description": $desc })
            }
        }
    };
}

string_schema!(ItemKind, "ItemKind", "skill, mcp, agent, plugin or extra");
string_schema!(ItemKey, "ItemKey", "`kind/name`");
string_schema!(ItemId, "ItemId", "`source:kind/name`");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_displays_item_id() {
        let id: ItemId = "payments-skills:skill/write-spec".parse().unwrap();
        assert_eq!(id.source(), "payments-skills");
        assert_eq!(id.key().kind(), ItemKind::Skill);
        assert_eq!(id.key().name(), "write-spec");
        assert_eq!(id.to_string(), "payments-skills:skill/write-spec");
    }

    #[test]
    fn rejects_malformed_ids() {
        for bad in [
            "",
            "payments-skills",
            "payments-skills:write-spec",
            "payments-skills:widget/write-spec",
            "Payments:skill/x",
            "a--b:skill/x",
            "src:skill/",
            "src:skill/../etc",
            "src:skill/a/b",
            "src:skill/.hidden",
            "src:skill/Upper",
            "src:skill/with space",
        ] {
            assert!(bad.parse::<ItemId>().is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn kind_round_trips() {
        for kind in ItemKind::ALL {
            assert_eq!(kind.as_str().parse::<ItemKind>().unwrap(), kind);
        }
    }

    #[test]
    fn serde_uses_string_form() {
        let key: ItemKey = "mcp/jira".parse().unwrap();
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, "\"mcp/jira\"");
        assert_eq!(serde_json::from_str::<ItemKey>(&json).unwrap(), key);
    }

    #[test]
    fn ids_order_by_source_then_kind_then_name() {
        let mut ids: Vec<ItemId> = ["b:skill/a", "a:skill/z", "a:mcp/z", "a:skill/b"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        ids.sort();
        let got: Vec<String> = ids.iter().map(ToString::to_string).collect();
        assert_eq!(got, ["a:skill/b", "a:skill/z", "a:mcp/z", "b:skill/a"]);
    }
}

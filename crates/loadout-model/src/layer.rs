//! Layers, ranks and profiles.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// A layer type with its rank (specificity); higher ranks win ties.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Layer {
    pub name: String,
    pub rank: i64,
}

/// Where a layer's rank, or a source's placement, came from.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum RankOrigin {
    /// Loadout's built-in ranks.
    Default,
    /// Suggested by a source's `LOADOUT.md` (`layers:`).
    Source { source: String },
    /// The company config's `company.layers`.
    CompanyConfig,
    /// `[layers]` in your `config.toml`.
    Config,
    /// A subscription's `layer` / `rank`: your `[[source]]` in
    /// `config.toml`, or (with `upstream_of`) the `upstream:` entry of the
    /// source that pulled it in.
    Subscription {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        upstream_of: Option<String>,
    },
}

impl std::fmt::Display for RankOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RankOrigin::Default => write!(f, "the built-in defaults"),
            RankOrigin::Source { source } => write!(f, "{source}'s LOADOUT.md"),
            RankOrigin::CompanyConfig => write!(f, "the company config"),
            RankOrigin::Config => write!(f, "your config.toml [layers]"),
            RankOrigin::Subscription { upstream_of: None } => {
                write!(f, "your [[source]] in config.toml")
            }
            RankOrigin::Subscription {
                upstream_of: Some(s),
            } => write!(f, "the upstream: entry in {s}"),
        }
    }
}

/// One layer's rank and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerRank {
    pub rank: i64,
    pub from: RankOrigin,
    /// The rank before your `config.toml` changed it: the one its sources
    /// or the company config declared. Locks are compared at this rank.
    /// `None` for a layer only your `config.toml` defines.
    pub declared: Option<i64>,
}

/// The layer ranks in effect. See `loadout_core::layers::merge` for how
/// defaults, sources, the company config and `config.toml` combine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerModel {
    ranks: BTreeMap<String, LayerRank>,
}

/// The personal layer: a person's own source.
pub const USER_LAYER: &str = "user";

/// The built-in ranks (company 0 … role 40, plus `project` at 35 and
/// `user` at 50).
pub const DEFAULT_LAYERS: [(&str, i64); 8] = [
    ("company", 0),
    ("org", 10),
    ("team", 20),
    ("product", 25),
    ("squad", 30),
    ("project", 35),
    ("role", 40),
    (USER_LAYER, 50),
];

impl Default for LayerModel {
    /// The built-in ranks, used when nothing else defines layers.
    fn default() -> Self {
        LayerModel::new(DEFAULT_LAYERS)
    }
}

impl LayerModel {
    /// Layers with the built-in origin.
    pub fn new<'a>(layers: impl IntoIterator<Item = (&'a str, i64)>) -> Self {
        let mut m = LayerModel {
            ranks: BTreeMap::new(),
        };
        for (n, r) in layers {
            m.declare(n, r, RankOrigin::Default);
        }
        m
    }

    /// Sets `layer`'s declared rank (a source's, the company config's or
    /// the default).
    pub fn declare(&mut self, layer: &str, rank: i64, from: RankOrigin) {
        self.ranks.insert(
            layer.to_owned(),
            LayerRank {
                rank,
                from,
                declared: Some(rank),
            },
        );
    }

    /// Re-ranks `layer` locally (`config.toml`), keeping its declared rank.
    pub fn override_rank(&mut self, layer: &str, rank: i64) {
        let declared = self.ranks.get(layer).and_then(|l| l.declared);
        self.ranks.insert(
            layer.to_owned(),
            LayerRank {
                rank,
                from: RankOrigin::Config,
                declared,
            },
        );
    }

    /// Adds the `user` layer above every other one, unless it's defined.
    pub fn with_user_layer(mut self) -> Self {
        if !self.ranks.contains_key(USER_LAYER) {
            let top = self.ranks.values().map(|l| l.rank).max().unwrap_or(0);
            self.declare(USER_LAYER, top + 10, RankOrigin::Default);
        }
        self
    }

    pub fn rank(&self, layer: &str) -> Option<i64> {
        self.ranks.get(layer).map(|l| l.rank)
    }

    /// The rank `layer`'s sources declared, before any local re-rank;
    /// the effective rank for a layer only `config.toml` defines.
    pub fn declared_rank(&self, layer: &str) -> Option<i64> {
        self.ranks.get(layer).map(|l| l.declared.unwrap_or(l.rank))
    }

    pub fn get(&self, layer: &str) -> Option<&LayerRank> {
        self.ranks.get(layer)
    }

    pub fn layers(&self) -> impl Iterator<Item = (&str, i64)> {
        self.ranks.iter().map(|(n, l)| (n.as_str(), l.rank))
    }

    /// Every layer with its rank and origin, by name.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &LayerRank)> {
        self.ranks.iter().map(|(n, l)| (n.as_str(), l))
    }
}

/// The groups a person belongs to, per layer. Multi-valued per layer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    groups: BTreeMap<String, BTreeSet<String>>,
}

impl Profile {
    pub fn add(&mut self, layer: impl Into<String>, group: impl Into<String>) {
        self.groups
            .entry(layer.into())
            .or_default()
            .insert(group.into());
    }

    pub fn contains(&self, layer: &str, group: &str) -> bool {
        self.groups.get(layer).is_some_and(|u| u.contains(group))
    }

    pub fn groups(&self, layer: &str) -> impl Iterator<Item = &str> {
        self.groups
            .get(layer)
            .into_iter()
            .flat_map(|u| u.iter().map(String::as_str))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.groups
            .iter()
            .flat_map(|(s, us)| us.iter().map(move |u| (s.as_str(), u.as_str())))
    }

    /// `applies_to` matching: every listed axis must share at least one
    /// group with the profile (AND across axes, OR within an axis).
    pub fn matches(&self, applies_to: &BTreeMap<String, Vec<String>>) -> Result<(), String> {
        for (axis, allowed) in applies_to {
            if !allowed.iter().any(|u| self.contains(axis, u)) {
                return Err(axis.clone());
            }
        }
        Ok(())
    }
}

impl<'a> FromIterator<(&'a str, &'a str)> for Profile {
    fn from_iter<T: IntoIterator<Item = (&'a str, &'a str)>>(iter: T) -> Self {
        let mut p = Profile::default();
        for (s, u) in iter {
            p.add(s, u);
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ranks() {
        let m = LayerModel::default();
        assert_eq!(m.rank("company"), Some(0));
        assert_eq!(m.rank("role"), Some(40));
        assert_eq!(m.rank("project"), Some(35));
        assert_eq!(m.rank("user"), Some(50));
        assert_eq!(m.rank("chapter"), None);
        let company_config = LayerModel::new([("company", 0), ("team", 70)]).with_user_layer();
        assert_eq!(company_config.rank("user"), Some(80));
        let own = LayerModel::new([("company", 0), ("user", 5)]).with_user_layer();
        assert_eq!(own.rank("user"), Some(5));
    }

    #[test]
    fn local_override_keeps_the_declared_rank() {
        let mut m = LayerModel::default();
        m.override_rank("company", 99);
        m.override_rank("beta", 27);
        assert_eq!(m.rank("company"), Some(99));
        assert_eq!(m.declared_rank("company"), Some(0));
        assert_eq!(m.get("company").unwrap().from, RankOrigin::Config);
        // Only config.toml knows `beta`: its declared rank is the local one.
        assert_eq!(m.declared_rank("beta"), Some(27));
        assert_eq!(m.get("beta").unwrap().declared, None);
    }

    #[test]
    fn applies_to_is_and_across_axes_or_within() {
        let p: Profile = [("role", "developer"), ("product", "billing")]
            .into_iter()
            .collect();
        let at = |pairs: &[(&str, &[&str])]| -> BTreeMap<String, Vec<String>> {
            pairs
                .iter()
                .map(|(a, us)| (a.to_string(), us.iter().map(|u| u.to_string()).collect()))
                .collect()
        };
        assert!(p.matches(&at(&[])).is_ok());
        assert!(p.matches(&at(&[("role", &["pm", "developer"])])).is_ok());
        assert!(
            p.matches(&at(&[("role", &["developer"]), ("product", &["billing"])]))
                .is_ok()
        );
        assert_eq!(
            p.matches(&at(&[("role", &["developer"]), ("product", &["payroll"])])),
            Err("product".into())
        );
        assert_eq!(p.matches(&at(&[("role", &["pm"])])), Err("role".into()));
    }
}

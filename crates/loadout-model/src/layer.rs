//! Layers, ranks and profiles.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// A layer type with its rank (specificity); higher ranks win ties.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Layer {
    pub name: String,
    pub rank: i64,
}

/// The layer ranks in effect: the company config's, or [`LayerModel::default`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerModel {
    ranks: BTreeMap<String, i64>,
}

/// The personal layer: a person's own source.
pub const USER_LAYER: &str = "user";

impl Default for LayerModel {
    /// The default ranks (company 0 … role 40) plus `project` at 35 and
    /// `user` at 50, used until a company config defines its own.
    fn default() -> Self {
        LayerModel::new([
            ("company", 0),
            ("org", 10),
            ("team", 20),
            ("product", 25),
            ("squad", 30),
            ("project", 35),
            ("role", 40),
            (USER_LAYER, 50),
        ])
    }
}

impl LayerModel {
    pub fn new<'a>(layers: impl IntoIterator<Item = (&'a str, i64)>) -> Self {
        LayerModel {
            ranks: layers.into_iter().map(|(n, r)| (n.to_owned(), r)).collect(),
        }
    }

    /// Adds the `user` layer above every other one, unless it's defined.
    pub fn with_user_layer(mut self) -> Self {
        if !self.ranks.contains_key(USER_LAYER) {
            let top = self.ranks.values().copied().max().unwrap_or(0);
            self.ranks.insert(USER_LAYER.to_owned(), top + 10);
        }
        self
    }

    pub fn rank(&self, layer: &str) -> Option<i64> {
        self.ranks.get(layer).copied()
    }

    pub fn layers(&self) -> impl Iterator<Item = (&str, i64)> {
        self.ranks.iter().map(|(n, r)| (n.as_str(), *r))
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

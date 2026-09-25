//! The resolution algorithm: from every candidate item in every
//! subscribed source to exactly one winner per `kind/name`, its enabled
//! state, and an explanation trail for `lo why`.
//!
//! [`resolve`] is a pure function. Its output does not depend on the order
//! of the candidates it is given.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use loadout_model::{ItemId, ItemKey, LayerModel, Mode, Profile};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::enabled::{EnabledReason, enabled_state};

/// One item from one source, with its effective metadata (item `loadout:`
/// block merged over source defaults).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: ItemId,
    pub layer: String,
    /// `None` when neither the item nor its source names a group; such
    /// items apply to everyone in the profile.
    pub group: Option<String>,
    pub applies_to: BTreeMap<String, Vec<String>>,
    pub mode: Option<Mode>,
    pub locked: bool,
    pub overridable: bool,
    /// Optional include-list of target ids.
    pub targets: Option<Vec<String>>,
    /// The source's tie-breaker priority; higher wins.
    pub priority: i64,
    pub content_hash: String,
}

/// Everything resolution depends on.
#[derive(Debug, Clone)]
pub struct Input<'a> {
    pub layers: &'a LayerModel,
    pub profile: &'a Profile,
    pub candidates: &'a [Candidate],
    /// User toggles, keyed by full item id.
    pub toggles: &'a BTreeMap<String, bool>,
    /// User preferences among equal-rank candidates: `kind/name` → source.
    pub prefer: &'a BTreeMap<String, String>,
    /// Resolve for this target (applies items' `targets` include-lists);
    /// `None` ignores them.
    pub target: Option<&'a str>,
}

/// Why a candidate did or did not win.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    Winner,
    /// Filtered: the profile doesn't contain the item's group.
    NotMember,
    /// Filtered: the item's layer isn't defined in the layer model.
    UnknownLayer,
    /// Filtered: `applies_to` didn't match on this axis.
    AppliesTo {
        axis: String,
    },
    /// Filtered: the item's `targets` list excludes the target.
    TargetExcluded,
    /// Lost to a more specific (higher-rank) candidate.
    Outranked {
        by: ItemId,
    },
    /// Tried to shadow a locked / non-overridable item from a more general
    /// layer (`blocked_override`).
    BlockedOverride {
        by: ItemId,
    },
    /// Lost to a locked / non-overridable candidate at a lower or equal rank.
    LostToLocked {
        by: ItemId,
    },
    /// Equal rank; the winner's source has a higher priority.
    LowerPriority {
        by: ItemId,
    },
    /// Equal rank; the user preferred the winner (`lo prefer`).
    NotPreferred {
        by: ItemId,
    },
    /// Equal rank and priority: an unresolved conflict, broken by source id.
    ConflictTieBreak {
        by: ItemId,
    },
}

impl Outcome {
    pub fn is_filtered(&self) -> bool {
        matches!(
            self,
            Outcome::NotMember
                | Outcome::UnknownLayer
                | Outcome::AppliesTo { .. }
                | Outcome::TargetExcluded
        )
    }
}

/// How the winner was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// It was the only eligible candidate.
    Only,
    /// The most general locked / non-overridable candidate wins outright.
    Locked,
    /// The most specific (highest-rank) candidate wins.
    HighestRank,
    /// Equal rank; higher source priority.
    Priority,
    /// Equal rank; the user's `lo prefer` choice (beats priority).
    Preferred,
    /// Equal rank and priority; lexicographically first source id.
    ConflictTieBreak,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Trail {
    pub id: ItemId,
    pub layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// `None` for layers missing from the layer model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<i64>,
    pub priority: i64,
    pub locked: bool,
    pub overridable: bool,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnabledBy {
    Required,
    Default,
    Toggle,
}

/// The result for one `kind/name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Resolved {
    pub key: ItemKey,
    /// `None` when every candidate was filtered out.
    pub winner: Option<ItemId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<Rule>,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_by: Option<EnabledBy>,
    /// Every candidate, winner first, then by id.
    pub candidates: Vec<Trail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Warning {
    /// A more specific layer tried to shadow a locked / non-overridable item.
    BlockedOverride {
        key: ItemKey,
        blocked: ItemId,
        by: ItemId,
    },
    /// Equal rank and priority; `picked` won by source id.
    Conflict {
        key: ItemKey,
        candidates: Vec<ItemId>,
        picked: ItemId,
    },
    /// The user toggled off an item that cannot be disabled.
    IgnoredToggle { id: ItemId },
    /// An item uses a layer the layer model doesn't define.
    UnknownLayer { id: ItemId, layer: String },
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Warning::BlockedOverride { key, blocked, by } => write!(
                f,
                "{blocked} cannot override {key}: {by} is locked or not overridable"
            ),
            Warning::Conflict {
                key,
                candidates,
                picked,
            } => {
                let ids: Vec<String> = candidates.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "conflict for {key} between {} (same rank and priority); using {picked}. \
                     Choose one with `lo prefer <source:{key}>`",
                    ids.join(", ")
                )
            }
            Warning::IgnoredToggle { id } => {
                write!(
                    f,
                    "{id} is required and cannot be disabled; ignoring your toggle"
                )
            }
            Warning::UnknownLayer { id, layer } => {
                write!(f, "{id} uses unknown layer {layer:?}; ignored")
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Resolution {
    /// One entry per `kind/name`, sorted by key.
    pub items: Vec<Resolved>,
    pub warnings: Vec<Warning>,
}

impl Resolution {
    pub fn get(&self, key: &ItemKey) -> Option<&Resolved> {
        self.items
            .binary_search_by(|r| r.key.cmp(key))
            .ok()
            .map(|i| &self.items[i])
    }

    pub fn has_conflicts(&self) -> bool {
        self.warnings
            .iter()
            .any(|w| matches!(w, Warning::Conflict { .. }))
    }
}

/// Runs the resolution steps: collect, filter, group, pick, enable, explain.
pub fn resolve(input: &Input<'_>) -> Resolution {
    let mut sorted: Vec<&Candidate> = input.candidates.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    sorted.dedup_by(|a, b| a.id == b.id);

    let mut groups: BTreeMap<&ItemKey, Vec<&Candidate>> = BTreeMap::new();
    for c in sorted {
        groups.entry(c.id.key()).or_default().push(c);
    }

    let mut out = Resolution::default();
    for (key, cands) in groups {
        out.items
            .push(resolve_group(input, key, &cands, &mut out.warnings));
    }
    out.warnings.sort_by_key(|w| w.to_string());
    out.warnings.dedup();
    out
}

fn resolve_group(
    input: &Input<'_>,
    key: &ItemKey,
    cands: &[&Candidate],
    warnings: &mut Vec<Warning>,
) -> Resolved {
    let mut trails: BTreeMap<&ItemId, Trail> = BTreeMap::new();
    let mut eligible: Vec<(&Candidate, i64)> = Vec::new();

    // Step 2: filter by membership, applies_to and target.
    for c in cands {
        let rank = input.layers.rank(&c.layer);
        let filtered = if rank.is_none() {
            warnings.push(Warning::UnknownLayer {
                id: c.id.clone(),
                layer: c.layer.clone(),
            });
            Some(Outcome::UnknownLayer)
        } else if c
            .group
            .as_deref()
            .is_some_and(|u| !input.profile.contains(&c.layer, u))
        {
            Some(Outcome::NotMember)
        } else if let Err(axis) = input.profile.matches(&c.applies_to) {
            Some(Outcome::AppliesTo { axis })
        } else if let (Some(t), Some(only)) = (input.target, &c.targets)
            && !only.iter().any(|x| x == t)
        {
            Some(Outcome::TargetExcluded)
        } else {
            None
        };
        trails.insert(
            &c.id,
            trail(c, rank, filtered.clone().unwrap_or(Outcome::Winner)),
        );
        if filtered.is_none() {
            eligible.push((c, rank.expect("checked above")));
        }
    }

    // Step 4: pick a winner.
    let decision = pick_winner(input, key, &eligible);
    let (winner, rule) = match &decision {
        Some((w, rule, losers)) => {
            for (id, outcome) in losers {
                if let Outcome::BlockedOverride { by } = outcome {
                    warnings.push(Warning::BlockedOverride {
                        key: key.clone(),
                        blocked: (*id).clone(),
                        by: by.clone(),
                    });
                }
                trails.get_mut(id).expect("trail exists").outcome = outcome.clone();
            }
            (Some(*w), Some(*rule))
        }
        None => (None, None),
    };
    if let Some((w, Rule::ConflictTieBreak, losers)) = &decision {
        let mut ids: Vec<ItemId> = losers
            .iter()
            .filter(|(_, o)| matches!(o, Outcome::ConflictTieBreak { .. }))
            .map(|(id, _)| (*id).clone())
            .collect();
        ids.push(w.id.clone());
        ids.sort();
        warnings.push(Warning::Conflict {
            key: key.clone(),
            candidates: ids,
            picked: w.id.clone(),
        });
    }

    // Step 5: enabled state.
    let (enabled, enabled_by) = match winner {
        Some(w) => {
            let toggle = input.toggles.get(&w.id.to_string()).copied();
            let state = enabled_state(w.mode, w.locked, toggle);
            if state.ignored_toggle {
                warnings.push(Warning::IgnoredToggle { id: w.id.clone() });
            }
            let by = match state.reason {
                EnabledReason::Required => EnabledBy::Required,
                EnabledReason::Default => EnabledBy::Default,
                EnabledReason::Toggle => EnabledBy::Toggle,
            };
            (state.enabled, Some(by))
        }
        None => (false, None),
    };

    // Step 6: explanation trail, winner first.
    let mut candidates: Vec<Trail> = trails.into_values().collect();
    candidates.sort_by(|a, b| {
        let aw = a.outcome == Outcome::Winner;
        let bw = b.outcome == Outcome::Winner;
        bw.cmp(&aw).then_with(|| a.id.cmp(&b.id))
    });

    Resolved {
        key: key.clone(),
        winner: winner.map(|w| w.id.clone()),
        rule,
        enabled,
        enabled_by,
        candidates,
    }
}

type Decision<'c> = (&'c Candidate, Rule, Vec<(&'c ItemId, Outcome)>);

fn pick_winner<'c>(
    input: &Input<'_>,
    key: &ItemKey,
    eligible: &[(&'c Candidate, i64)],
) -> Option<Decision<'c>> {
    if eligible.is_empty() {
        return None;
    }
    let preferred = input.prefer.get(&key.to_string()).map(String::as_str);

    let locking: Vec<&(&Candidate, i64)> = eligible
        .iter()
        .filter(|(c, _)| c.locked || !c.overridable)
        .collect();

    if let Some(min_rank) = locking.iter().map(|(_, r)| *r).min() {
        // Step 4.1: the most general locking candidate wins outright.
        let pool: Vec<(&Candidate, i64)> = locking
            .iter()
            .filter(|(_, r)| *r == min_rank)
            .map(|x| **x)
            .collect();
        let (winner, tie_rule, mut losers) = break_tie(&pool, preferred);
        let rule = if pool.len() == 1 {
            Rule::Locked
        } else {
            tie_rule
        };
        for (c, r) in eligible {
            if c.id == winner.id || pool.iter().any(|(p, _)| p.id == c.id) {
                continue;
            }
            let by = winner.id.clone();
            losers.push((
                &c.id,
                if *r > min_rank {
                    Outcome::BlockedOverride { by }
                } else {
                    Outcome::LostToLocked { by }
                },
            ));
        }
        return Some((winner, rule, losers));
    }

    // Step 4.2: the most specific candidate wins.
    let max_rank = eligible.iter().map(|(_, r)| *r).max().expect("non-empty");
    let pool: Vec<(&Candidate, i64)> = eligible
        .iter()
        .filter(|(_, r)| *r == max_rank)
        .copied()
        .collect();
    let (winner, tie_rule, mut losers) = break_tie(&pool, preferred);
    let rule = if eligible.len() == 1 {
        Rule::Only
    } else if pool.len() == 1 {
        Rule::HighestRank
    } else {
        tie_rule
    };
    for (c, r) in eligible {
        if *r < max_rank {
            losers.push((
                &c.id,
                Outcome::Outranked {
                    by: winner.id.clone(),
                },
            ));
        }
    }
    Some((winner, rule, losers))
}

/// Step 4.3: equal rank → the user's preference (`lo prefer`, which acts
/// as a priority bump for that one item), then source priority, then the
/// lexicographically first source id (a conflict).
fn break_tie<'c>(
    pool: &[(&'c Candidate, i64)],
    preferred: Option<&str>,
) -> (&'c Candidate, Rule, Vec<(&'c ItemId, Outcome)>) {
    let mut ordered: Vec<&Candidate> = pool.iter().map(|(c, _)| *c).collect();
    ordered.sort_by(|a, b| {
        is_pref(b, preferred)
            .cmp(&is_pref(a, preferred))
            .then_with(|| b.priority.cmp(&a.priority))
            .then_with(|| a.id.source().cmp(b.id.source()))
    });
    let winner = ordered[0];
    let mut rule = Rule::Only;
    let mut losers = Vec::new();
    for c in &ordered[1..] {
        let by = winner.id.clone();
        let outcome = match c.priority.cmp(&winner.priority) {
            _ if is_pref(winner, preferred) => {
                rule = rule_max(rule, Rule::Preferred);
                Outcome::NotPreferred { by }
            }
            Ordering::Less => {
                rule = rule_max(rule, Rule::Priority);
                Outcome::LowerPriority { by }
            }
            _ => {
                rule = Rule::ConflictTieBreak;
                Outcome::ConflictTieBreak { by }
            }
        };
        losers.push((&c.id, outcome));
    }
    (winner, rule, losers)
}

fn is_pref(c: &Candidate, preferred: Option<&str>) -> bool {
    preferred == Some(c.id.source())
}

/// The rule reported for a tie is the "weakest" one needed to decide it.
fn rule_max(a: Rule, b: Rule) -> Rule {
    let order = |r: Rule| match r {
        Rule::Only | Rule::Locked | Rule::HighestRank => 0,
        Rule::Priority => 1,
        Rule::Preferred => 3,
        Rule::ConflictTieBreak => 2,
    };
    if order(b) > order(a) { b } else { a }
}

fn trail(c: &Candidate, rank: Option<i64>, outcome: Outcome) -> Trail {
    Trail {
        id: c.id.clone(),
        layer: c.layer.clone(),
        group: c.group.clone(),
        rank,
        priority: c.priority,
        locked: c.locked,
        overridable: c.overridable,
        outcome,
    }
}

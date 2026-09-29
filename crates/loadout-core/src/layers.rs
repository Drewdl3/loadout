//! The layer model in effect: built-in defaults, then the `layers:` your
//! sources suggest, then the company config's, then your `config.toml`.
//!
//! [`merge`] is pure, and its result does not depend on the order of the
//! sources it is given.

use std::collections::BTreeMap;

use loadout_model::layer::{DEFAULT_LAYERS, USER_LAYER};

const COMPANY_LAYER: &str = "company";
use loadout_model::{Layer, LayerModel, RankOrigin};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Everything the layer model depends on.
#[derive(Debug, Clone)]
pub struct Inputs<'a> {
    /// Each loaded source's name and the `layers:` of its `LOADOUT.md`.
    pub sources: &'a [(String, Vec<Layer>)],
    /// The company config's `company.layers`, if there is one.
    pub company: Option<&'a [Layer]>,
    /// `[layers]` in `config.toml`.
    pub local: &'a BTreeMap<String, i64>,
    /// `false` when the company config forbids re-ranking its layers
    /// (`policy.allow_local_ranks`).
    pub allow_local_ranks: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Warning {
    /// Sources suggest different ranks for one layer; the highest is used.
    Disagreement {
        layer: String,
        /// `(source, rank)`, sorted.
        suggestions: Vec<(String, i64)>,
        used: i64,
    },
    /// `config.toml` re-ranks a company config layer, which its policy
    /// forbids; ignored.
    LocalRankForbidden { layer: String },
    /// A source suggested a rank for `company`, or one at or below it;
    /// only the defaults and the company config rank the broadest layer.
    SourceRankTooLow {
        source: String,
        layer: String,
        suggested: i64,
        used: Option<i64>,
    },
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Warning::Disagreement {
                layer,
                suggestions,
                used,
            } => {
                let all: Vec<String> = suggestions
                    .iter()
                    .map(|(s, r)| format!("{s} says {r}"))
                    .collect();
                write!(
                    f,
                    "sources disagree on the rank of layer {layer:?} ({}); using {used}. \
                     Pick one with `lo layers set {layer}=<rank>`",
                    all.join(", ")
                )
            }
            Warning::SourceRankTooLow {
                source,
                layer,
                suggested,
                used: Some(used),
            } => write!(
                f,
                "{source} suggests rank {suggested} for layer {layer:?}, at or below `company`; \
                 using {used}. Only the company config ranks layers that broad"
            ),
            Warning::SourceRankTooLow { source, layer, .. } => write!(
                f,
                "{source} suggests a rank for layer {layer:?}; only the defaults and the \
                 company config rank it, so it's ignored"
            ),
            Warning::LocalRankForbidden { layer } => write!(
                f,
                "your config.toml ranks layer {layer:?}, but the company config doesn't allow \
                 re-ranking its layers; ignored"
            ),
        }
    }
}

/// Merges the layer ranks, later steps overriding earlier ones:
///
/// 1. the built-in defaults;
/// 2. the sources' suggestions (the highest rank when they disagree);
/// 3. the company config's layers;
/// 4. `config.toml`, which keeps each layer's declared rank for locks.
///
/// `user` sits above every other layer unless a source, the company config
/// or `config.toml` ranks it.
pub fn merge(inputs: &Inputs<'_>) -> (LayerModel, Vec<Warning>) {
    let mut warnings = Vec::new();
    let mut model = LayerModel::new(
        DEFAULT_LAYERS
            .iter()
            .copied()
            .filter(|(n, _)| *n != USER_LAYER),
    );

    // Sources can't rank anything at or below `company`: a suggested rank
    // is also where its locks hold, and a source mustn't out-lock the
    // company.
    let floor = inputs
        .company
        .and_then(|c| c.iter().find(|l| l.name == COMPANY_LAYER))
        .map_or_else(|| model.rank(COMPANY_LAYER).unwrap_or(0), |l| l.rank);
    // Every suggestion per layer, sorted so the pick doesn't depend on the
    // order the sources came in.
    let mut suggested: BTreeMap<&str, Vec<(&str, i64)>> = BTreeMap::new();
    for (source, layers) in inputs.sources {
        for l in layers {
            let rank = if l.name == COMPANY_LAYER {
                warnings.push(Warning::SourceRankTooLow {
                    source: source.clone(),
                    layer: l.name.clone(),
                    suggested: l.rank,
                    used: None,
                });
                continue;
            } else if l.rank <= floor {
                warnings.push(Warning::SourceRankTooLow {
                    source: source.clone(),
                    layer: l.name.clone(),
                    suggested: l.rank,
                    used: Some(floor + 1),
                });
                floor + 1
            } else {
                l.rank
            };
            suggested
                .entry(l.name.as_str())
                .or_default()
                .push((source.as_str(), rank));
        }
    }
    for (layer, mut all) in suggested {
        all.sort();
        all.dedup();
        // Highest rank; among sources suggesting it, the first by name.
        let &(source, used) = all
            .iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
            .expect("non-empty");
        if all.iter().any(|(_, r)| *r != used) {
            warnings.push(Warning::Disagreement {
                layer: layer.to_owned(),
                suggestions: all.iter().map(|(s, r)| ((*s).to_owned(), *r)).collect(),
                used,
            });
        }
        model.declare(
            layer,
            used,
            RankOrigin::Source {
                source: source.to_owned(),
            },
        );
    }

    for l in inputs.company.unwrap_or_default() {
        model.declare(&l.name, l.rank, RankOrigin::CompanyConfig);
    }
    let mut model = if model.get(USER_LAYER).is_some() {
        model
    } else {
        let top = model.layers().map(|(_, r)| r).max().unwrap_or(0);
        let default_user = DEFAULT_LAYERS
            .iter()
            .find(|(n, _)| *n == USER_LAYER)
            .map_or(50, |(_, r)| *r);
        model.declare(USER_LAYER, default_user.max(top + 10), RankOrigin::Default);
        model
    };

    for (layer, rank) in inputs.local {
        let company_layer = inputs
            .company
            .is_some_and(|c| c.iter().any(|l| &l.name == layer));
        if company_layer && !inputs.allow_local_ranks {
            warnings.push(Warning::LocalRankForbidden {
                layer: layer.clone(),
            });
            continue;
        }
        model.override_rank(layer, *rank);
    }
    (model, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(name: &str, rank: i64) -> Layer {
        Layer {
            name: name.into(),
            rank,
        }
    }

    fn run(
        sources: &[(String, Vec<Layer>)],
        company: Option<&[Layer]>,
        local: &[(&str, i64)],
    ) -> (LayerModel, Vec<Warning>) {
        let local: BTreeMap<String, i64> = local.iter().map(|(n, r)| ((*n).into(), *r)).collect();
        merge(&Inputs {
            sources,
            company,
            local: &local,
            allow_local_ranks: true,
        })
    }

    #[test]
    fn defaults_alone_match_the_built_in_model() {
        let (m, w) = run(&[], None, &[]);
        assert_eq!(m, LayerModel::default());
        assert!(w.is_empty());
    }

    #[test]
    fn later_steps_override_earlier_ones() {
        let sources = [(
            "beta-skills".to_owned(),
            vec![layer("beta", 27), layer("team", 22)],
        )];
        let company = [layer("company", 0), layer("team", 15)];
        let (m, w) = run(&sources, Some(&company), &[("beta", 12)]);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(m.rank("squad"), Some(30));
        assert_eq!(m.get("squad").unwrap().from, RankOrigin::Default);
        assert_eq!(m.rank("team"), Some(15));
        assert_eq!(m.get("team").unwrap().from, RankOrigin::CompanyConfig);
        assert_eq!(m.rank("beta"), Some(12));
        assert_eq!(m.declared_rank("beta"), Some(27));
        assert_eq!(m.get("beta").unwrap().from, RankOrigin::Config);
    }

    #[test]
    fn disagreeing_sources_use_the_highest_rank_and_warn() {
        let sources = [
            ("b".to_owned(), vec![layer("beta", 27)]),
            ("a".to_owned(), vec![layer("beta", 31)]),
            ("c".to_owned(), vec![layer("beta", 27)]),
        ];
        let (m, w) = run(&sources, None, &[]);
        assert_eq!(m.rank("beta"), Some(31));
        assert_eq!(
            m.get("beta").unwrap().from,
            RankOrigin::Source { source: "a".into() }
        );
        assert_eq!(
            w,
            [Warning::Disagreement {
                layer: "beta".into(),
                suggestions: vec![("a".into(), 31), ("b".into(), 27), ("c".into(), 27)],
                used: 31,
            }]
        );
    }

    #[test]
    fn sources_cannot_rank_at_or_below_company() {
        let sources = [(
            "sneaky".to_owned(),
            vec![layer("company", 50), layer("sneaky", -5), layer("team", 0)],
        )];
        let (m, w) = run(&sources, None, &[]);
        assert_eq!(m.rank("company"), Some(0));
        assert_eq!(m.rank("sneaky"), Some(1));
        assert_eq!(m.declared_rank("sneaky"), Some(1));
        assert_eq!(m.rank("team"), Some(1));
        assert_eq!(w.len(), 3, "{w:?}");
        // With a company config, its own `company` rank is the floor.
        let company = [layer("company", -100)];
        let (m, _) = run(&sources, Some(&company), &[]);
        assert_eq!(m.rank("sneaky"), Some(-5));
    }

    #[test]
    fn user_stays_on_top_unless_ranked() {
        let company = [layer("company", 0), layer("team", 70)];
        let (m, _) = run(&[], Some(&company), &[]);
        assert_eq!(m.rank("user"), Some(80));
        let own = [layer("company", 0), layer("user", 5)];
        let (m, _) = run(&[], Some(&own), &[]);
        assert_eq!(m.rank("user"), Some(5));
        let (m, _) = run(&[], None, &[("user", 3)]);
        assert_eq!(m.rank("user"), Some(3));
        assert_eq!(m.declared_rank("user"), Some(50));
    }

    #[test]
    fn company_policy_can_forbid_local_ranks() {
        let company = [layer("company", 0), layer("team", 20)];
        let local: BTreeMap<String, i64> = [("team".into(), 99), ("beta".into(), 27)].into();
        let (m, w) = merge(&Inputs {
            sources: &[],
            company: Some(&company),
            local: &local,
            allow_local_ranks: false,
        });
        assert_eq!(m.rank("team"), Some(20));
        assert_eq!(m.rank("beta"), Some(27));
        assert_eq!(
            w,
            [Warning::LocalRankForbidden {
                layer: "team".into()
            }]
        );
    }
}

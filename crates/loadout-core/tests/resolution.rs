//! Resolution matrix and property tests.

use std::collections::BTreeMap;

use loadout_core::resolve::{
    Candidate, EnabledBy, Input, Outcome, Resolution, Rule, Warning, resolve,
};
use loadout_model::{ItemId, LayerModel, Mode, Profile};
use proptest::prelude::*;

/// A candidate `source:skill/<name>` at `layer`/`group`.
fn cand(source: &str, name: &str, layer: &str, group: &str) -> Candidate {
    Candidate {
        id: format!("{source}:skill/{name}").parse().unwrap(),
        layer: layer.into(),
        group: Some(group.into()),
        applies_to: BTreeMap::new(),
        mode: Some(Mode::DefaultOn),
        locked: false,
        overridable: true,
        targets: None,
        priority: 0,
        content_hash: format!("blake3:{source}"),
    }
}

fn developer() -> Profile {
    [
        ("company", "acme"),
        ("org", "eng"),
        ("team", "payments-dev"),
        ("team", "platform"),
        ("product", "billing"),
        ("role", "developer"),
    ]
    .into_iter()
    .collect()
}

struct Case {
    layers: LayerModel,
    profile: Profile,
    toggles: BTreeMap<String, bool>,
    prefer: BTreeMap<String, String>,
    target: Option<&'static str>,
}

impl Case {
    fn new() -> Self {
        Case {
            layers: LayerModel::default(),
            profile: developer(),
            toggles: BTreeMap::new(),
            prefer: BTreeMap::new(),
            target: None,
        }
    }

    fn run(&self, candidates: &[Candidate]) -> Resolution {
        resolve(&Input {
            layers: &self.layers,
            profile: &self.profile,
            candidates,
            toggles: &self.toggles,
            prefer: &self.prefer,
            target: self.target,
        })
    }
}

fn id(s: &str) -> ItemId {
    s.parse().unwrap()
}

fn outcome_of<'r>(r: &'r Resolution, name: &str, cand: &str) -> &'r Outcome {
    let item = r.get(&format!("skill/{name}").parse().unwrap()).unwrap();
    &item
        .candidates
        .iter()
        .find(|t| t.id == id(cand))
        .unwrap()
        .outcome
}

fn winner(r: &Resolution, name: &str) -> Option<String> {
    r.get(&format!("skill/{name}").parse().unwrap())
        .unwrap()
        .winner
        .as_ref()
        .map(ToString::to_string)
}

// same name at company (default-on) and team → team wins.
#[test]
fn more_specific_layer_wins() {
    let r = Case::new().run(&[
        cand("acme", "x", "company", "acme"),
        cand("payments", "x", "team", "payments-dev"),
    ]);
    assert_eq!(winner(&r, "x").as_deref(), Some("payments:skill/x"));
    let item = &r.items[0];
    assert_eq!(item.rule, Some(Rule::HighestRank));
    assert!(item.enabled);
    assert_eq!(
        outcome_of(&r, "x", "acme:skill/x"),
        &Outcome::Outranked {
            by: id("payments:skill/x")
        }
    );
    assert!(r.warnings.is_empty());
}

// company locked + team override → company wins, blocked_override warning.
#[test]
fn locked_general_item_blocks_override() {
    let mut company = cand("acme", "x", "company", "acme");
    company.locked = true;
    let r = Case::new().run(&[company, cand("payments", "x", "team", "payments-dev")]);
    assert_eq!(winner(&r, "x").as_deref(), Some("acme:skill/x"));
    assert_eq!(r.items[0].rule, Some(Rule::Locked));
    assert_eq!(r.items[0].enabled_by, Some(EnabledBy::Required));
    assert_eq!(
        outcome_of(&r, "x", "payments:skill/x"),
        &Outcome::BlockedOverride {
            by: id("acme:skill/x")
        }
    );
    assert_eq!(
        r.warnings,
        [Warning::BlockedOverride {
            key: "skill/x".parse().unwrap(),
            blocked: id("payments:skill/x"),
            by: id("acme:skill/x"),
        }]
    );
}

// company overridable:false + role override → company wins.
#[test]
fn non_overridable_general_item_wins() {
    let mut company = cand("acme", "x", "company", "acme");
    company.overridable = false;
    let r = Case::new().run(&[company, cand("devs", "x", "role", "developer")]);
    assert_eq!(winner(&r, "x").as_deref(), Some("acme:skill/x"));
    assert_eq!(r.items[0].rule, Some(Rule::Locked));
    // Not locked, so it is not forced on.
    assert_eq!(r.items[0].enabled_by, Some(EnabledBy::Default));
    assert!(matches!(r.warnings[0], Warning::BlockedOverride { .. }));
}

#[test]
fn most_general_locking_candidate_wins_among_several() {
    let mut org = cand("eng", "x", "org", "eng");
    org.locked = true;
    let mut team = cand("payments", "x", "team", "payments-dev");
    team.overridable = false;
    let company = cand("acme", "x", "company", "acme");
    let r = Case::new().run(&[team, org, company]);
    assert_eq!(winner(&r, "x").as_deref(), Some("eng:skill/x"));
    assert_eq!(
        outcome_of(&r, "x", "acme:skill/x"),
        &Outcome::LostToLocked {
            by: id("eng:skill/x")
        }
    );
    assert_eq!(
        outcome_of(&r, "x", "payments:skill/x"),
        &Outcome::BlockedOverride {
            by: id("eng:skill/x")
        }
    );
}

// two team-rank sources, different priority → higher priority wins.
#[test]
fn equal_rank_higher_priority_wins() {
    let mut a = cand("alpha", "x", "team", "payments-dev");
    let mut b = cand("beta", "x", "team", "platform");
    a.priority = 1;
    b.priority = 5;
    let r = Case::new().run(&[a, b]);
    assert_eq!(winner(&r, "x").as_deref(), Some("beta:skill/x"));
    assert_eq!(r.items[0].rule, Some(Rule::Priority));
    assert_eq!(
        outcome_of(&r, "x", "alpha:skill/x"),
        &Outcome::LowerPriority {
            by: id("beta:skill/x")
        }
    );
    assert!(!r.has_conflicts());
}

// equal priority → conflict warning, deterministic pick.
#[test]
fn equal_rank_equal_priority_is_a_conflict() {
    let r = Case::new().run(&[
        cand("zeta", "x", "team", "platform"),
        cand("alpha", "x", "team", "payments-dev"),
    ]);
    assert_eq!(winner(&r, "x").as_deref(), Some("alpha:skill/x"));
    assert_eq!(r.items[0].rule, Some(Rule::ConflictTieBreak));
    assert!(r.has_conflicts());
    let w = r.warnings[0].to_string();
    assert!(w.contains("lo prefer <source:skill/x>"), "{w}");
}

#[test]
fn prefer_resolves_conflict_and_beats_priority() {
    let mut case = Case::new();
    case.prefer.insert("skill/x".into(), "zeta".into());
    let mut alpha = cand("alpha", "x", "team", "payments-dev");
    let r = case.run(&[alpha.clone(), cand("zeta", "x", "team", "platform")]);
    assert_eq!(winner(&r, "x").as_deref(), Some("zeta:skill/x"));
    assert_eq!(r.items[0].rule, Some(Rule::Preferred));
    assert!(!r.has_conflicts());

    alpha.priority = 10;
    let r = case.run(&[alpha, cand("zeta", "x", "team", "platform")]);
    assert_eq!(winner(&r, "x").as_deref(), Some("zeta:skill/x"));
}

// applies_to {role: [pm]} excluded for a developer profile.
#[test]
fn applies_to_excludes_non_matching_profile() {
    let mut pm = cand("acme", "x", "company", "acme");
    pm.applies_to.insert("role".into(), vec!["pm".into()]);
    let r = Case::new().run(&[pm]);
    assert_eq!(winner(&r, "x"), None);
    assert!(!r.items[0].enabled);
    assert_eq!(
        outcome_of(&r, "x", "acme:skill/x"),
        &Outcome::AppliesTo {
            axis: "role".into()
        }
    );
}

#[test]
fn filtered_specific_item_falls_back_to_general_one() {
    let mut pm_only = cand("payments", "x", "team", "payments-dev");
    pm_only.applies_to.insert("role".into(), vec!["pm".into()]);
    let r = Case::new().run(&[pm_only, cand("acme", "x", "company", "acme")]);
    assert_eq!(winner(&r, "x").as_deref(), Some("acme:skill/x"));
    assert_eq!(r.items[0].rule, Some(Rule::Only));
}

#[test]
fn non_member_groups_are_filtered() {
    let r = Case::new().run(&[
        cand("other", "x", "team", "not-my-team"),
        cand("acme", "y", "company", "acme"),
    ]);
    assert_eq!(winner(&r, "x"), None);
    assert_eq!(outcome_of(&r, "x", "other:skill/x"), &Outcome::NotMember);
    assert_eq!(winner(&r, "y").as_deref(), Some("acme:skill/y"));
}

#[test]
fn groupless_items_apply_to_everyone() {
    let mut c = cand("acme", "x", "team", "ignored");
    c.group = None;
    let r = Case::new().run(&[c]);
    assert_eq!(winner(&r, "x").as_deref(), Some("acme:skill/x"));
}

#[test]
fn unknown_layer_is_filtered_with_warning() {
    let r = Case::new().run(&[cand("acme", "x", "chapter", "acme")]);
    assert_eq!(winner(&r, "x"), None);
    assert!(matches!(r.warnings[0], Warning::UnknownLayer { .. }));
}

// required item + user toggle off → stays enabled, warning.
#[test]
fn required_item_ignores_toggle_off() {
    let mut case = Case::new();
    case.toggles.insert("acme:skill/x".into(), false);
    let mut c = cand("acme", "x", "company", "acme");
    c.mode = Some(Mode::Required);
    let r = case.run(&[c]);
    assert!(r.items[0].enabled);
    assert_eq!(r.items[0].enabled_by, Some(EnabledBy::Required));
    assert_eq!(
        r.warnings,
        [Warning::IgnoredToggle {
            id: id("acme:skill/x")
        }]
    );
}

// default-off + toggle on → enabled.
#[test]
fn default_off_with_toggle_on_is_enabled() {
    let mut case = Case::new();
    let mut c = cand("acme", "x", "company", "acme");
    c.mode = Some(Mode::DefaultOff);
    let r = case.run(std::slice::from_ref(&c));
    assert!(!r.items[0].enabled);
    case.toggles.insert("acme:skill/x".into(), true);
    let r = case.run(&[c]);
    assert!(r.items[0].enabled);
    assert_eq!(r.items[0].enabled_by, Some(EnabledBy::Toggle));
}

#[test]
fn toggle_on_a_loser_does_not_matter() {
    let mut case = Case::new();
    case.toggles.insert("payments:skill/x".into(), false);
    let r = case.run(&[
        cand("acme", "x", "company", "acme"),
        cand("payments", "x", "team", "payments-dev"),
    ]);
    // The toggle targets the winner by id, so it disables it.
    assert!(!r.items[0].enabled);
    case.toggles.clear();
    case.toggles.insert("acme:skill/x".into(), false);
    let r = case.run(&[
        cand("acme", "x", "company", "acme"),
        cand("payments", "x", "team", "payments-dev"),
    ]);
    assert!(
        r.items[0].enabled,
        "toggling a loser leaves the winner alone"
    );
}

// item targets: [pi] absent from Claude Code output.
#[test]
fn target_include_list() {
    let mut case = Case::new();
    let mut pi_only = cand("payments", "x", "team", "payments-dev");
    pi_only.targets = Some(vec!["pi".into()]);
    let company = cand("acme", "x", "company", "acme");

    case.target = Some("claude-code");
    let r = case.run(&[pi_only.clone(), company.clone()]);
    assert_eq!(winner(&r, "x").as_deref(), Some("acme:skill/x"));
    assert_eq!(
        outcome_of(&r, "x", "payments:skill/x"),
        &Outcome::TargetExcluded
    );
    let r = case.run(std::slice::from_ref(&pi_only));
    assert_eq!(winner(&r, "x"), None);

    case.target = Some("pi");
    let r = case.run(&[pi_only, company]);
    assert_eq!(winner(&r, "x").as_deref(), Some("payments:skill/x"));
}

#[test]
fn trail_lists_winner_first_then_by_id() {
    let r = Case::new().run(&[
        cand("b-src", "x", "company", "acme"),
        cand("a-src", "x", "org", "eng"),
        cand("c-src", "x", "role", "developer"),
    ]);
    let ids: Vec<String> = r.items[0]
        .candidates
        .iter()
        .map(|t| t.id.to_string())
        .collect();
    assert_eq!(ids, ["c-src:skill/x", "a-src:skill/x", "b-src:skill/x"]);
}

fn arb_candidate() -> impl Strategy<Value = Candidate> {
    let sources = prop::sample::select(vec!["s-a", "s-b", "s-c", "s-d"]);
    let names = prop::sample::select(vec!["x", "y"]);
    let layers = prop::sample::select(vec![
        ("company", "acme"),
        ("org", "eng"),
        ("team", "payments-dev"),
        ("team", "platform"),
        ("team", "other"),
        ("role", "developer"),
        ("nope", "x"),
    ]);
    let modes = prop::option::of(prop::sample::select(vec![
        Mode::Required,
        Mode::DefaultOn,
        Mode::DefaultOff,
    ]));
    let targets = prop::option::of(prop::sample::select(vec![
        vec!["pi".to_owned()],
        vec!["claude-code".to_owned()],
    ]));
    (
        sources,
        names,
        layers,
        modes,
        any::<bool>(),
        any::<bool>(),
        0i64..3,
        targets,
        any::<bool>(),
    )
        .prop_map(
            |(src, name, (layer, group), mode, locked, overridable, priority, targets, pm)| {
                let mut c = cand(src, name, layer, group);
                c.mode = mode;
                c.locked = locked;
                c.overridable = overridable;
                c.priority = priority;
                c.targets = targets;
                if pm {
                    c.applies_to.insert("role".into(), vec!["pm".into()]);
                }
                c
            },
        )
}

proptest! {
    // permuting source order never changes output.
    #[test]
    fn output_is_independent_of_candidate_order(
        cands in prop::collection::vec(arb_candidate(), 0..12),
        seed in any::<u64>(),
    ) {
        let mut case = Case::new();
        case.target = Some("claude-code");
        case.toggles.insert("s-a:skill/x".into(), false);
        case.prefer.insert("skill/y".into(), "s-c".into());

        // Candidate ids are unique per source+name; keep the first of each.
        let mut seen = std::collections::BTreeSet::new();
        let cands: Vec<Candidate> = cands.into_iter().filter(|c| seen.insert(c.id.clone())).collect();

        let base = case.run(&cands);
        let mut shuffled = cands.clone();
        // Deterministic Fisher–Yates driven by `seed`.
        let mut state = seed | 1;
        for i in (1..shuffled.len()).rev() {
            state ^= state << 13; state ^= state >> 7; state ^= state << 17;
            shuffled.swap(i, (state % (i as u64 + 1)) as usize);
        }
        prop_assert_eq!(&case.run(&shuffled), &base);
        let mut reversed = cands;
        reversed.reverse();
        prop_assert_eq!(&case.run(&reversed), &base);
    }

    #[test]
    fn every_group_has_at_most_one_winner_and_a_full_trail(
        cands in prop::collection::vec(arb_candidate(), 0..12),
    ) {
        let mut seen = std::collections::BTreeSet::new();
        let cands: Vec<Candidate> = cands.into_iter().filter(|c| seen.insert(c.id.clone())).collect();
        let r = Case::new().run(&cands);
        let total: usize = r.items.iter().map(|i| i.candidates.len()).sum();
        prop_assert_eq!(total, cands.len());
        for item in &r.items {
            let winners = item.candidates.iter().filter(|t| t.outcome == Outcome::Winner).count();
            prop_assert_eq!(winners, usize::from(item.winner.is_some()));
            if item.winner.is_none() {
                prop_assert!(!item.enabled);
                prop_assert!(item.candidates.iter().all(|t| t.outcome.is_filtered()));
            }
        }
    }
}

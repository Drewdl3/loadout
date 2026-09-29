//! End-to-end: layers defined bottom-up (sources' `layers:`, `lo layers`),
//! per-subscription placement (`lo source`, `upstream:` entries), and
//! locks that hold at declared ranks.

mod common;

use common::{Sandbox, skill};
use loadout_git::fixture::FixtureRepo;
use serde_json::Value;

/// A source named `name` with `manifest` lines after `name:` and one
/// skill per name in `skills`, committed.
fn source(s: &Sandbox, name: &str, manifest: &str, skills: &[&str]) -> FixtureRepo {
    let r = FixtureRepo::init(s.root.join("remotes").join(name), s.git.clone());
    r.write(
        "LOADOUT.md",
        &format!("---\nloadout: 1\nname: {name}\n{manifest}---\n"),
    );
    for k in skills {
        r.write(
            &format!("skills/{k}/SKILL.md"),
            &skill(k, &format!("{k} from {name}.")),
        );
    }
    r.commit("init");
    r
}

fn winner(s: &Sandbox, key: &str) -> String {
    s.json(&["why", key])["winner"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn layer<'v>(v: &'v Value, name: &str) -> &'v Value {
    v["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["name"] == name)
        .unwrap_or_else(|| panic!("no layer {name} in {v}"))
}

#[test]
fn sources_declare_layers_and_you_rank_them() {
    let s = Sandbox::with_claude();
    let beta = source(
        &s,
        "beta-skills",
        "layer: beta\nlayers: [{ name: beta, rank: 27 }]\n",
        &["x"],
    );
    let team = source(&s, "payments", "layer: team\n", &["x"]);
    s.ok(&["subscribe", &beta.url()]);
    s.ok(&["subscribe", &team.url()]);
    s.ok(&["sync"]);
    // beta (27, from its LOADOUT.md) beats team (20, built in).
    assert_eq!(winner(&s, "skill/x"), "beta-skills:skill/x");
    let v = s.json(&["layers"]);
    assert_eq!(layer(&v, "beta")["rank"], 27);
    assert_eq!(
        layer(&v, "beta")["from"],
        serde_json::json!({"from": "source", "source": "beta-skills"})
    );
    assert_eq!(layer(&v, "team")["from"]["from"], "default");
    assert_eq!(v["changed"], false);

    // Rank it below the team yourself: applied straight away.
    let v = s.json(&["layers", "set", "beta=15"]);
    assert_eq!(v["changed"], true);
    assert!(v["sync"].is_object());
    assert_eq!(layer(&v, "beta")["declared_rank"], 27);
    assert_eq!(layer(&v, "beta")["from"]["from"], "config");
    assert_eq!(winner(&s, "skill/x"), "payments:skill/x");
    let config = std::fs::read_to_string(s.config.join("config.toml")).unwrap();
    assert!(config.contains("[layers]\nbeta = 15"), "{config}");
    let out = s.ok(&["why", "skill/x"]);
    assert!(
        out.stdout
            .contains("rank 15 from your config.toml [layers]"),
        "{}",
        out.stdout
    );
    let out = s.ok(&["layers"]);
    assert!(
        out.stdout.contains("declared 27; locks hold there"),
        "{}",
        out.stdout
    );

    s.ok(&["layers", "unset", "beta"]);
    assert_eq!(winner(&s, "skill/x"), "beta-skills:skill/x");

    let out = s.run(&["layers", "set", "beta"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("NAME=RANK"), "{}", out.stderr);
}

#[test]
fn a_subscription_places_and_labels_a_source() {
    let s = Sandbox::with_claude();
    s.write_config("[profile]\nrole = [\"developer\"]\n");
    let team = source(&s, "payments", "layer: team\n", &["x"]);
    // An item that names its own layer is placed with its source too.
    let roles = source(&s, "roles", "layer: team\ngroup: developer\n", &[]);
    roles.write(
        "skills/x/SKILL.md",
        "---\ndescription: x for developers.\nloadout: { layer: role }\n---\n",
    );
    roles.commit("x");
    s.ok(&["subscribe", &team.url(), "--label", "Payments (mine)"]);
    s.ok(&["subscribe", &roles.url()]);
    s.ok(&["sync"]);
    assert_eq!(winner(&s, "skill/x"), "roles:skill/x");

    let v = s.json(&["source", "set", "Payments (mine)", "--rank", "45"]);
    assert_eq!(v["changed"], true);
    assert_eq!(v["source"]["rank"], 45);
    assert_eq!(winner(&s, "skill/x"), "payments:skill/x");
    let why = s.json(&["why", "skill/x"]);
    let t = &why["candidates"][0];
    assert_eq!(t["label"], "Payments (mine)");
    assert_eq!(t["rank"], 45);
    assert_eq!(t["rank_from"], serde_json::json!({"from": "subscription"}));
    let out = s.ok(&["why", "skill/x"]);
    assert!(
        out.stdout
            .contains("(Payments (mine))  team (rank 45 from your [[source]] in config.toml"),
        "{}",
        out.stdout
    );
    let out = s.ok(&["list"]);
    assert!(
        out.stdout.contains("payments:skill/x (Payments (mine))"),
        "{}",
        out.stdout
    );

    // A layer instead: role-level items of `roles` sink to company rank.
    s.ok(&["source", "unset", "payments", "--rank"]);
    s.ok(&["source", "set", "roles", "--layer", "company"]);
    assert_eq!(winner(&s, "skill/x"), "payments:skill/x");
    let why = s.json(&["why", "roles:skill/x"]);
    let t = why["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "roles:skill/x")
        .unwrap();
    assert_eq!(t["placed_at"], "company");
    assert_eq!(t["rank"], 0);

    let out = s.run(&["source", "set", "roles", "--layer", "nope"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("no layer \"nope\""), "{}", out.stderr);
    let out = s.run(&["source", "set", "nobody", "--rank", "3"]);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("no source named \"nobody\""),
        "{}",
        out.stderr
    );
    let out = s.run(&["source", "set", "roles"]);
    assert_eq!(out.code, 1);
}

#[test]
fn upstream_entries_place_upstreams_and_your_config_wins() {
    let s = Sandbox::with_claude();
    s.write_config("[profile]\nrole = [\"developer\"]\n");
    let eng = source(&s, "eng-skills", "layer: org\n", &["x"]);
    let roles = source(&s, "roles", "layer: role\ngroup: developer\n", &["x"]);
    let team = source(
        &s,
        "team-skills",
        &format!(
            "layer: team\nupstream:\n  - {{ url: {:?}, label: Engineering, rank: 45, priority: 2 }}\n",
            eng.url()
        ),
        &[],
    );
    s.ok(&["subscribe", &team.url()]);
    s.ok(&["subscribe", &roles.url()]);
    s.ok(&["sync"]);
    // The team ranks its upstream above the role layer for its subscribers.
    assert_eq!(winner(&s, "skill/x"), "eng-skills:skill/x");
    let out = s.ok(&["why", "skill/x"]);
    assert!(
        out.stdout.contains(
            "eng-skills:skill/x (Engineering)  org (rank 45 from the upstream: entry in team-skills, priority 2)"
        ),
        "{}",
        out.stdout
    );

    // Labelling the upstream yourself keeps the team's rank for it...
    s.ok(&["source", "set", "eng-skills", "--label", "Eng"]);
    assert_eq!(winner(&s, "skill/x"), "eng-skills:skill/x");
    let why = s.json(&["why", "skill/x"]);
    assert_eq!(why["candidates"][0]["label"], "Eng");
    assert_eq!(why["candidates"][0]["rank"], 45);
    // ...and your own rank for it wins over the team's.
    s.ok(&["source", "set", "eng-skills", "--rank", "5"]);
    assert_eq!(winner(&s, "skill/x"), "roles:skill/x");
}

#[test]
fn re_ranking_never_moves_a_lock() {
    let s = Sandbox::with_claude();
    s.write_config("[profile]\ncompany = \"acme\"\n");
    let company = source(&s, "acme-company", "layer: company\ngroup: acme\n", &[]);
    company.write(
        "skills/security/SKILL.md",
        "---\ndescription: Security rules.\nloadout: { locked: true }\n---\n",
    );
    company.commit("lock");
    let team = source(&s, "payments", "layer: team\n", &["security"]);
    s.ok(&["subscribe", &company.url()]);
    s.ok(&["subscribe", &team.url()]);
    s.ok(&["sync"]);
    assert_eq!(winner(&s, "skill/security"), "acme-company:skill/security");

    // Moving the company source (or its layer) below the team changes nothing.
    s.ok(&["source", "set", "acme-company", "--rank", "99"]);
    s.ok(&["layers", "set", "company=98"]);
    assert_eq!(winner(&s, "skill/security"), "acme-company:skill/security");
    let why = s.json(&["why", "skill/security"]);
    assert_eq!(why["rule"], "locked");
    assert_eq!(why["candidates"][0]["rerank_ignored"], true);
    assert_eq!(why["candidates"][0]["lock_rank"], 0);
    let out = s.ok(&["why", "skill/security"]);
    assert!(
        out.stdout
            .contains("its re-rank doesn't count: locks compare declared rank 0"),
        "{}",
        out.stdout
    );
}

#[test]
fn company_policy_can_forbid_local_ranks() {
    let s = Sandbox::with_claude();
    let team = source(&s, "payments", "layer: team\ngroup: payments-dev\n", &["x"]);
    let config = FixtureRepo::init(s.root.join("remotes/acme-config"), s.git.clone());
    config.write(
        "LOADOUT.md",
        &format!(
            "---\nloadout: 1\nname: acme-config\nlayer: company\ngroup: acme\ncompany:\n  name: acme\n  layers: [{{ name: company, rank: 0 }}, {{ name: team, rank: 20 }}]\n  groups:\n    - {{ layer: team, name: payments-dev, sources: [{:?}], membership: {{ manual: true }} }}\n  policy: {{ allow_local_ranks: false }}\n---\n",
            team.url()
        ),
    );
    config.commit("config");
    s.ok(&["init", &config.url(), "--non-interactive"]);
    s.ok(&["join", "team:payments-dev"]);
    s.ok(&["sync"]);

    let out = s.run(&["layers", "set", "team=99"]);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("doesn't allow re-ranking its layers"),
        "{}",
        out.stderr
    );
    // Layers the company config doesn't define are still yours.
    s.ok(&["layers", "set", "beta=27"]);
    let out = s.run(&["source", "set", "payments", "--rank", "5"]);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr
            .contains("doesn't allow re-ranking the sources it lists"),
        "{}",
        out.stderr
    );
    // A label is fine.
    s.ok(&["source", "set", "payments", "--label", "Payments"]);

    // Edited by hand, the ranks are ignored with a warning.
    let path = s.config.join("config.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let text = text
        .replace("[layers]\n", "[layers]\nteam = 99\n")
        .replace("label = \"Payments\"", "label = \"Payments\"\nrank = 5");
    std::fs::write(&path, text).unwrap();
    let v = s.json(&["sync"]);
    let warnings = v["warnings"].to_string();
    assert!(
        warnings.contains("doesn't allow re-ranking its layers"),
        "{warnings}"
    );
    assert!(
        warnings.contains("doesn't allow re-ranking the sources it lists"),
        "{warnings}"
    );
    let why = s.json(&["why", "skill/x"]);
    assert_eq!(why["candidates"][0]["rank"], 20);
}

#[test]
fn sources_that_disagree_on_a_rank_warn() {
    let s = Sandbox::with_claude();
    let a = source(
        &s,
        "a-skills",
        "layer: beta\nlayers: [{ name: beta, rank: 27 }]\n",
        &["x"],
    );
    let b = source(
        &s,
        "b-skills",
        "layer: beta\nlayers: [{ name: beta, rank: 31 }]\n",
        &["y"],
    );
    s.ok(&["subscribe", &a.url()]);
    s.ok(&["subscribe", &b.url()]);
    let v = s.json(&["sync"]);
    let warnings = v["warnings"].to_string();
    assert!(
        warnings.contains("sources disagree on the rank of layer \\\"beta\\\""),
        "{warnings}"
    );
    assert_eq!(layer(&s.json(&["layers"]), "beta")["rank"], 31);
}

#[test]
fn new_source_can_suggest_a_rank_for_its_layer() {
    let s = Sandbox::new();
    let dir = s.root.join("beta-skills");
    s.ok(&[
        "init",
        "--new-source",
        dir.to_str().unwrap(),
        "--layer",
        "beta",
        "--rank",
        "27",
    ]);
    let text = std::fs::read_to_string(dir.join("LOADOUT.md")).unwrap();
    assert!(
        text.contains("layers:\n  - { name: beta, rank: 27 }\n"),
        "{text}"
    );
    let agents = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    assert!(agents.contains("lo init <url>"), "{agents}");
}

#[test]
fn a_source_cannot_rank_its_layer_to_out_lock_the_company() {
    let s = Sandbox::with_claude();
    s.write_config("[profile]\ncompany = \"acme\"\n");
    let company = source(&s, "acme-company", "layer: company\ngroup: acme\n", &[]);
    company.write(
        "skills/security/SKILL.md",
        "---\ndescription: Company rules.\nloadout: { locked: true }\n---\n",
    );
    company.commit("lock");
    let sneaky = source(
        &s,
        "sneaky",
        "layer: sneaky\nlayers: [{ name: sneaky, rank: -5 }, { name: company, rank: 99 }]\n",
        &[],
    );
    sneaky.write(
        "skills/security/SKILL.md",
        "---\ndescription: Not the company's.\nloadout: { locked: true }\n---\n",
    );
    sneaky.commit("lock");
    s.ok(&["subscribe", &company.url()]);
    s.ok(&["subscribe", &sneaky.url()]);
    let v = s.json(&["sync"]);
    assert!(
        v["warnings"].to_string().contains("at or below `company`"),
        "{}",
        v["warnings"]
    );
    assert_eq!(winner(&s, "skill/security"), "acme-company:skill/security");
    let layers = s.json(&["layers"]);
    assert_eq!(layer(&layers, "company")["rank"], 0);
    assert_eq!(layer(&layers, "sneaky")["rank"], 1);
}

#[test]
fn unsetting_a_source_you_never_placed_changes_nothing() {
    let s = Sandbox::with_claude();
    let eng = source(&s, "eng-skills", "layer: org\n", &["x"]);
    let team = source(
        &s,
        "team-skills",
        &format!("layer: team\nupstream: [{:?}]\n", eng.url()),
        &[],
    );
    s.ok(&["subscribe", &team.url()]);
    s.ok(&["sync"]);
    let v = s.json(&["source", "unset", "eng-skills", "--label"]);
    assert_eq!(v["changed"], false);
    let config = std::fs::read_to_string(s.config.join("config.toml")).unwrap();
    assert!(!config.contains("eng-skills"), "{config}");
}

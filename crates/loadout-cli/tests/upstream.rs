//! Upstream sources: a source's `LOADOUT.md` names higher
//! sources under `upstream:`, and subscribing to it installs theirs too.

mod common;

use common::{Sandbox, skill};
use loadout_git::fixture::FixtureRepo;
use serde_json::Value;

fn repo(s: &Sandbox, name: &str, manifest_extra: &str, layer: &str, group: &str) -> FixtureRepo {
    let r = FixtureRepo::init(s.root.join("remotes").join(name), s.git.clone());
    r.write(
        "LOADOUT.md",
        &format!(
            "---\nloadout: 1\nname: {name}\nlayer: {layer}\ngroup: {group}\n{manifest_extra}---\n"
        ),
    );
    r
}

fn installed(s: &Sandbox, name: &str) -> Option<String> {
    std::fs::read_to_string(s.skills_dir().join(name).join("SKILL.md")).ok()
}

fn source<'a>(sync: &'a Value, name: &str) -> &'a Value {
    sync["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == name)
        .unwrap_or_else(|| panic!("no source {name} in {sync:#}"))
}

/// company ← eng (org) ← checkout-squad; the user subscribes to the squad
/// only. The squad names eng with a relative URL, eng names the company
/// with an absolute one.
fn chain() -> (Sandbox, FixtureRepo, FixtureRepo, FixtureRepo) {
    let s = Sandbox::with_claude();
    let company = repo(&s, "acme-company", "", "company", "acme");
    company
        .write(
            "skills/security/SKILL.md",
            "---\ndescription: Security rules.\nloadout: { locked: true }\n---\n",
        )
        .write(
            "skills/pr-guide/SKILL.md",
            &skill("pr-guide", "Company PR guide."),
        );
    company.commit("company");
    let eng = repo(
        &s,
        "eng-skills",
        &format!("upstream:\n  - {:?}\n", company.url()),
        "org",
        "eng",
    );
    eng.write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "Eng spec."),
    );
    eng.commit("eng");
    let squad = repo(
        &s,
        "checkout-squad",
        "upstream: [../eng-skills]\n",
        "squad",
        "checkout",
    );
    squad
        .write(
            "skills/write-spec/SKILL.md",
            &skill("write-spec", "Checkout spec."),
        )
        .write(
            "skills/security/SKILL.md",
            &skill("security", "Squad tries to override."),
        );
    squad.commit("squad");
    (s, company, eng, squad)
}

#[test]
fn subscribing_to_a_source_pulls_in_its_upstreams() {
    let (s, _company, _eng, squad) = chain();
    s.ok(&["subscribe", &squad.url()]);
    let sync = s.json(&["sync"]);

    assert!(source(&sync, "checkout-squad")["via"].is_null());
    assert_eq!(source(&sync, "eng-skills")["via"], "checkout-squad");
    assert_eq!(source(&sync, "acme-company")["via"], "eng-skills");

    // Items from all three, resolved across the layers.
    assert!(
        installed(&s, "write-spec")
            .unwrap()
            .contains("Checkout spec.")
    );
    assert!(
        installed(&s, "pr-guide")
            .unwrap()
            .contains("Company PR guide.")
    );
    assert!(
        installed(&s, "security")
            .unwrap()
            .contains("Security rules.")
    );
    let why = s.json(&["why", "skill/security"]);
    assert_eq!(why["winner"], "acme-company:skill/security");

    // Human output says where a source came from.
    let out = s.ok(&["sync"]);
    assert!(
        out.stdout.contains("(upstream of eng-skills)"),
        "{}",
        out.stdout
    );

    // Everything is pinned: a second machine with only config + lock gets
    // the same result with `--locked`.
    let lock = std::fs::read_to_string(s.data.join("loadout.lock")).unwrap();
    for name in ["acme-company", "eng-skills", "checkout-squad"] {
        assert!(lock.contains(&format!("name = \"{name}\"")), "{lock}");
    }
    let fp = s.json(&["status"])["fingerprint"].clone();
    for dir in ["repos", "store"] {
        std::fs::remove_dir_all(s.data.join(dir)).unwrap();
    }
    std::fs::remove_dir_all(s.skills_dir()).unwrap();
    s.ok(&["sync", "--locked"]);
    assert!(installed(&s, "pr-guide").is_some());
    assert_eq!(s.json(&["status"])["fingerprint"], fp);

    // resolved.json records the graph for the UI.
    let resolved: Value =
        serde_json::from_str(&std::fs::read_to_string(s.data.join("resolved.json")).unwrap())
            .unwrap();
    let squad_entry = resolved["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "checkout-squad")
        .unwrap();
    assert!(
        squad_entry["upstream"][0]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with("remotes/eng-skills")
    );

    // Unsubscribing drops the whole chain.
    s.ok(&["unsubscribe", &squad.url()]);
    s.ok(&["sync"]);
    assert!(installed(&s, "pr-guide").is_none());
}

#[test]
fn cycles_and_missing_upstreams_do_not_break_sync() {
    let s = Sandbox::with_claude();
    let a = repo(
        &s,
        "a-skills",
        "upstream: [../b-skills, ../nowhere]\n",
        "team",
        "a",
    );
    a.write("skills/from-a/SKILL.md", &skill("from-a", "A."));
    a.commit("a");
    let b = repo(&s, "b-skills", "upstream: [../a-skills]\n", "team", "b");
    b.write("skills/from-b/SKILL.md", &skill("from-b", "B."));
    b.commit("b");
    s.ok(&["subscribe", &a.url()]);
    let out = s.run(&["sync", "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sync: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(sync["sources"].as_array().unwrap().len(), 2, "{sync:#}");
    let warnings = sync["warnings"].to_string();
    assert!(
        warnings.contains("nowhere") && warnings.contains("skipped"),
        "{warnings}"
    );
    assert!(installed(&s, "from-a").is_some() && installed(&s, "from-b").is_some());
}

#[test]
fn company_config_policy_applies_to_upstreams() {
    let s = Sandbox::with_claude();
    let eng = repo(&s, "eng-skills", "", "org", "eng");
    eng.write("skills/eng-only/SKILL.md", &skill("eng-only", "Eng."));
    eng.commit("eng");
    let stray = repo(&s, "stray-skills", "", "team", "stray");
    stray.write("skills/stray/SKILL.md", &skill("stray", "Stray."));
    stray.commit("stray");
    let team = repo(
        &s,
        "payments-skills",
        &format!("upstream: [{:?}, ../stray-skills]\n", eng.url()),
        "team",
        "payments-dev",
    );
    team.write("skills/team/SKILL.md", &skill("team", "Team."));
    team.commit("team");
    let company_config = FixtureRepo::init(s.root.join("remotes/acme-config"), s.git.clone());
    company_config.write(
        "LOADOUT.md",
        &format!(
            "---\nloadout: 1\nname: acme-config\nlayer: company\ngroup: acme\ncompany:\n  name: acme\n  layers:\n    - {{ name: company, rank: 0 }}\n    - {{ name: org, rank: 10 }}\n    - {{ name: team, rank: 20 }}\n  groups:\n    - layer: org\n      name: eng\n      sources: [{:?}]\n      membership: {{ env: {{ ACME_ORG: eng }} }}\n    - layer: team\n      name: payments-dev\n      sources: [{:?}]\n      membership: {{ env: {{ ACME_TEAM: payments-dev }} }}\n  policy:\n    allow_manual_sources: false\n---\n",
            eng.url(),
            team.url()
        ),
    );
    company_config.commit("company_config");
    // A member of the team but not (by discovery) of org:eng.
    let env = [("ACME_TEAM", "payments-dev")];
    let out = s.run_env(
        &["init", &company_config.url(), "--non-interactive", "--json"],
        &env,
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let init: Value = serde_json::from_str(&out.stdout).unwrap();
    let sync = &init["sync"];
    // The company config lists eng, so the team's upstream brings it in at org:eng.
    assert_eq!(source(sync, "eng-skills")["via"], "payments-skills");
    assert!(installed(&s, "eng-only").is_some());
    // stray-skills isn't in the company config, and manual sources aren't allowed.
    assert!(installed(&s, "stray").is_none());
    assert!(
        sync["warnings"].to_string().contains("stray-skills"),
        "{sync:#}"
    );
}

#[test]
fn shortcodes_pin_upstreams_too() {
    let (a, company, _eng, squad) = chain();
    a.ok(&["subscribe", &squad.url()]);
    a.ok(&["sync"]);
    let code = a.json(&["export"])["code"].as_str().unwrap().to_owned();
    let fp = a.json(&["status"])["fingerprint"].clone();
    // Upstream moves on; the importer still gets the exported commits.
    company.write("skills/pr-guide/SKILL.md", &skill("pr-guide", "Changed."));
    company.commit("later");

    let b = Sandbox::with_claude();
    b.ok(&["import", &code, "--yes"]);
    assert_eq!(b.json(&["status"])["fingerprint"], fp);
    assert!(
        installed(&b, "pr-guide")
            .unwrap()
            .contains("Company PR guide.")
    );
}

#[test]
fn dry_run_previews_a_source_and_its_upstreams() {
    let (s, _company, _eng, squad) = chain();
    let p = s.json(&["subscribe", &squad.url(), "--dry-run"]);
    let names: Vec<&str> = p["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["checkout-squad", "eng-skills", "acme-company"]);
    assert_eq!(p["sources"][2]["via"], "eng-skills");
    assert_eq!(
        p["sources"][0]["items"][0]["id"],
        "checkout-squad:skill/security"
    );
    // Nothing was subscribed or installed.
    assert!(!s.config.join("config.toml").exists());
    assert!(!s.skills_dir().join("security").exists());
    let human = s.ok(&["subscribe", &squad.url(), "--dry-run"]).stdout;
    assert!(human.contains("(upstream of checkout-squad)"), "{human}");
    let bad = s.run(&["subscribe", "/nowhere/at/all", "--dry-run"]);
    assert_eq!(bad.code, 1);
}

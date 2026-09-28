//! Nested sources: a team repo whose `LOADOUT.md` lists `nested:`
//! directories holding a `LOADOUT.md` per squad. One subscription brings
//! the team and its squads; you get the items of the squads you're in.

mod common;

use common::{Sandbox, skill};
use loadout_git::fixture::FixtureRepo;
use serde_json::Value;

/// team (payments) with squads/beaver and squads/west/alpha.
fn team(s: &Sandbox) -> FixtureRepo {
    let r = FixtureRepo::init(s.root.join("remotes").join("team"), s.git.clone());
    r.write(
        "LOADOUT.md",
        "---\nloadout: 1\nname: payments-team\nlayer: team\ngroup: payments\nnested: [squads]\n---\n",
    )
    .write(
        "skills/guide/SKILL.md",
        &skill("guide", "Team guide."),
    )
    .write(
        "skills/review/SKILL.md",
        &skill("review", "Team review."),
    )
    .write(
        "squads/beaver/LOADOUT.md",
        "---\nloadout: 1\nname: beaver-squad\nlayer: squad\ngroup: beaver\ndescription: Beaver squad.\n---\n",
    )
    .write(
        "squads/beaver/skills/guide/SKILL.md",
        &skill("guide", "Beaver guide."),
    )
    .write(
        "squads/beaver/skills/dam/SKILL.md",
        &skill("dam", "Beaver dam."),
    )
    .write(
        "squads/west/alpha/LOADOUT.md",
        "---\nloadout: 1\nname: alpha-squad\nlayer: squad\ngroup: alpha\n---\n",
    )
    .write(
        "squads/west/alpha/skills/probe/SKILL.md",
        &skill("probe", "Alpha probe."),
    );
    r.commit("team and squads");
    r
}

fn installed(s: &Sandbox, name: &str) -> Option<String> {
    std::fs::read_to_string(s.skills_dir().join(name).join("SKILL.md")).ok()
}

fn source<'a>(v: &'a Value, name: &str) -> &'a Value {
    v["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == name)
        .unwrap_or_else(|| panic!("no source {name} in {v:#}"))
}

#[test]
fn one_repo_holds_a_team_and_its_squads() {
    let s = Sandbox::with_claude();
    let repo = team(&s);
    s.write_config("[profile]\nteam = [\"payments\"]\nsquad = [\"beaver\"]\n");
    s.ok(&["subscribe", &repo.url()]);
    let sync = s.json(&["sync"]);

    // Each manifest is a source; the nested ones say where they live.
    assert!(source(&sync, "payments-team")["path"].is_null());
    assert_eq!(source(&sync, "payments-team")["items"], 2);
    assert_eq!(source(&sync, "beaver-squad")["path"], "squads/beaver");
    assert_eq!(source(&sync, "beaver-squad")["items"], 2);
    assert_eq!(source(&sync, "alpha-squad")["path"], "squads/west/alpha");
    let commit = &source(&sync, "payments-team")["commit"];
    assert_eq!(&source(&sync, "beaver-squad")["commit"], commit);

    // Your squad's items, over the team's; not other squads'.
    assert!(installed(&s, "guide").unwrap().contains("Beaver guide."));
    assert!(installed(&s, "dam").is_some());
    assert!(installed(&s, "review").is_some());
    assert!(installed(&s, "probe").is_none());
    let why = s.json(&["why", "skill/guide"]);
    assert_eq!(why["winner"], "beaver-squad:skill/guide");
    let why = s.json(&["why", "skill/probe"]);
    assert!(why["winner"].is_null(), "{why:#}");

    // Human output names the folder.
    let out = s.ok(&["sync"]);
    assert!(out.stdout.contains("(in squads/beaver/)"), "{}", out.stdout);

    // `lo info` reads the nested manifest.
    let info = s.json(&["info", "beaver-squad"]);
    assert_eq!(info["description"], "Beaver squad.");
    assert_eq!(info["layer"], "squad");

    // One repo, one lock entry; its items are pinned under their sources.
    let lock = std::fs::read_to_string(s.data.join("loadout.lock")).unwrap();
    assert_eq!(lock.matches("[[source]]").count(), 1, "{lock}");
    assert!(lock.contains("beaver-squad:skill/dam"), "{lock}");
    let fp = s.json(&["status"])["fingerprint"].clone();
    for dir in ["repos", "store"] {
        std::fs::remove_dir_all(s.data.join(dir)).unwrap();
    }
    std::fs::remove_dir_all(s.skills_dir()).unwrap();
    s.ok(&["sync", "--locked"]);
    assert!(installed(&s, "dam").is_some());
    assert_eq!(s.json(&["status"])["fingerprint"], fp);
}

#[test]
fn moving_squads_changes_what_you_get() {
    let s = Sandbox::with_claude();
    let repo = team(&s);
    s.write_config("[profile]\nteam = [\"payments\"]\nsquad = [\"alpha\"]\n");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);
    assert!(installed(&s, "probe").is_some());
    assert!(installed(&s, "dam").is_none());
    assert!(installed(&s, "guide").unwrap().contains("Team guide."));
}

#[test]
fn subscribing_does_not_join_the_squads() {
    // No [profile]: subscribing makes you a member of the team's group
    // only, so no squad's items apply.
    let s = Sandbox::with_claude();
    let repo = team(&s);
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);
    assert!(installed(&s, "review").is_some());
    assert!(installed(&s, "guide").unwrap().contains("Team guide."));
    assert!(installed(&s, "dam").is_none());
    assert!(installed(&s, "probe").is_none());
}

#[test]
fn preview_lists_nested_sources() {
    let s = Sandbox::with_claude();
    let repo = team(&s);
    let v = s.json(&["subscribe", "--dry-run", &repo.url()]);
    let names: Vec<&str> = v["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["payments-team", "beaver-squad", "alpha-squad"]);
    let beaver = source(&v, "beaver-squad");
    assert_eq!(beaver["path"], "squads/beaver");
    assert_eq!(beaver["layer"], "squad");
    assert_eq!(beaver["items"].as_array().unwrap().len(), 2);
}

#[test]
fn a_nested_name_taken_by_another_repo_is_dropped() {
    let s = Sandbox::with_claude();
    let other = s.source("beaver-squad");
    other.write("skills/other/SKILL.md", &skill("other", "Other."));
    other.commit("other");
    let repo = team(&s);
    s.write_config("[profile]\nteam = [\"payments\", \"payments-dev\"]\nsquad = [\"beaver\"]\n");
    // Repos load in order of their root names: the standalone
    // beaver-squad comes before payments-team, so the nested one is dropped.
    s.ok(&["subscribe", &other.url()]);
    s.ok(&["subscribe", &repo.url()]);
    let sync = s.json(&["sync"]);
    let warnings = sync["warnings"].to_string();
    assert!(
        warnings.contains("both call themselves \\\"beaver-squad\\\""),
        "{warnings}"
    );
    let beavers: Vec<&Value> = sync["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["name"] == "beaver-squad")
        .collect();
    assert_eq!(beavers.len(), 1, "{sync:#}");
    assert!(beavers[0]["path"].is_null(), "{sync:#}");
    assert!(installed(&s, "other").is_some());
    assert!(installed(&s, "dam").is_none());
    assert!(installed(&s, "review").is_some());
}

/// With a company config: the team repo is listed under the team's group,
/// the squad group only decides membership. The repo updates as one, so a
/// squad change is held for review unless every layer in the repo is
/// auto-applied, even though the team layer is.
#[test]
fn company_config_squads_and_review() {
    let s = Sandbox::with_claude();
    let repo = team(&s);
    let company = FixtureRepo::init(s.root.join("remotes/acme-config"), s.git.clone());
    company.write(
        "LOADOUT.md",
        &format!(
            "---\nloadout: 1\nname: acme-config\nlayer: company\ngroup: acme\ncompany:\n  name: acme\n  layers:\n    - {{ name: company, rank: 0 }}\n    - {{ name: team, rank: 20 }}\n    - {{ name: squad, rank: 30 }}\n  groups:\n    - layer: team\n      name: payments\n      sources: [{:?}]\n      membership: {{ env: {{ ACME_TEAM: payments }} }}\n    - layer: squad\n      name: beaver\n      membership: {{ env: {{ ACME_SQUAD: beaver }} }}\n    - layer: squad\n      name: alpha\n      membership: {{ env: {{ ACME_SQUAD: alpha }} }}\n  policy:\n    auto_apply: [company, team]\n---\n",
            repo.url()
        ),
    );
    company.commit("config");
    let env = [("ACME_TEAM", "payments"), ("ACME_SQUAD", "beaver")];
    let run = |args: &[&str]| s.run_env(args, &env);
    let out = run(&["init", &company.url(), "--non-interactive"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(installed(&s, "guide").unwrap().contains("Beaver guide."));
    assert!(installed(&s, "dam").is_some());
    assert!(installed(&s, "probe").is_none());

    // A squad change shares its kind/name with a team item; it's reported
    // under its full id and waits for review.
    repo.write(
        "squads/beaver/skills/guide/SKILL.md",
        &skill("guide", "Beaver guide v2."),
    );
    repo.commit("beaver v2");
    let out = run(&["sync"]);
    assert_eq!(out.code, 2, "pending → exit 2\n{}", out.stderr);
    assert!(installed(&s, "guide").unwrap().contains("Beaver guide."));
    let diff = run(&["diff", "payments-team"]);
    assert!(
        diff.stdout.contains("changed: beaver-squad:skill/guide"),
        "{}",
        diff.stdout
    );
    assert!(
        !diff.stdout.contains("changed: skill/guide"),
        "{}",
        diff.stdout
    );
    let out = run(&["approve", "payments-team"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(installed(&s, "guide").unwrap().contains("Beaver guide v2."));
}

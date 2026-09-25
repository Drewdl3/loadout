//! End-to-end: several sources at different layers, `why`, `prefer`,
//! `enable` / `disable`, conflicts.

mod common;

use common::{Sandbox, skill};
use loadout_git::fixture::FixtureRepo;

fn read(p: &std::path::Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

/// A source at `layer`/`group`.
fn source(s: &Sandbox, name: &str, layer: &str, group: &str) -> FixtureRepo {
    let repo =
        loadout_git::fixture::FixtureRepo::init(s.root.join("remotes").join(name), s.git.clone());
    repo.write(
        "LOADOUT.md",
        &format!("---\nloadout: 1\nname: {name}\nlayer: {layer}\ngroup: {group}\n---\n"),
    );
    repo
}

const PROFILE: &str =
    "[profile]\ncompany = \"acme\"\nteam = [\"payments-dev\"]\nrole = [\"developer\"]\n";

/// company (acme) + team (payments-dev) sources, subscribed.
fn layered() -> (Sandbox, FixtureRepo, FixtureRepo) {
    let s = Sandbox::with_claude();
    s.write_config(PROFILE);
    let company = source(&s, "acme-company", "company", "acme");
    company
        .write(
            "skills/write-spec/SKILL.md",
            &skill("write-spec", "Company spec format."),
        )
        .write(
            "skills/security/SKILL.md",
            "---\ndescription: Security rules.\nloadout: { locked: true }\n---\n",
        )
        .write(
            "skills/pm-helper/SKILL.md",
            "---\ndescription: For PMs.\nloadout: { applies_to: { role: [pm] } }\n---\n",
        );
    company.commit("company");
    let team = source(&s, "payments", "team", "payments-dev");
    team.write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "Payments spec format."),
    )
    .write(
        "skills/security/SKILL.md",
        &skill("security", "Team tries to override."),
    );
    team.commit("team");
    s.ok(&["subscribe", &company.url()]);
    s.ok(&["subscribe", &team.url()]);
    (s, company, team)
}

#[test]
fn layered_sources_resolve_by_rank() {
    let (s, _company, _team) = layered();
    let out = s.ok(&["sync"]);
    s.insta().bind(|| {
        insta::assert_snapshot!("layered_sync_stdout", out.stdout);
        insta::assert_snapshot!("layered_sync_stderr", out.stderr);
    });
    // Team wins write-spec; locked company security wins; pm-helper excluded.
    assert!(read(&s.skills_dir().join("write-spec/SKILL.md")).contains("Payments"));
    assert!(read(&s.skills_dir().join("security/SKILL.md")).contains("Security rules."));
    assert!(!s.skills_dir().join("pm-helper").exists());
}

#[test]
fn why_explains_each_rule() {
    let (s, _company, _team) = layered();
    s.ok(&["sync"]);
    for (name, item) in [
        ("why_team_wins", "skill/write-spec"),
        ("why_locked", "skill/security"),
        ("why_filtered", "skill/pm-helper"),
    ] {
        let out = s.ok(&["why", item]);
        s.insta().bind(|| insta::assert_snapshot!(name, out.stdout));
    }
    let json = s.json(&["why", "payments:skill/write-spec"]);
    s.insta()
        .bind(|| insta::assert_json_snapshot!("why_team_wins_json", json));

    let out = s.run(&["why", "skill/nope"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("no source provides skill/nope"));
}

#[test]
fn disable_and_enable() {
    let (s, _company, _team) = layered();
    s.ok(&["sync"]);

    // Locked items cannot be disabled.
    let out = s.run(&["disable", "skill/security"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("cannot be disabled"), "{}", out.stderr);

    // Normal items can; it applies immediately without fetching.
    let report = s.json(&["disable", "skill/write-spec"]);
    assert_eq!(
        report["targets"][0]["removed"],
        serde_json::json!(["skill/write-spec"])
    );
    assert!(report["sources"][0]["cached"].as_bool().unwrap());
    assert!(!s.skills_dir().join("write-spec").exists());
    let config = read(&s.config.join("config.toml"));
    assert!(
        config.contains("\"payments:skill/write-spec\" = false"),
        "{config}"
    );

    s.ok(&["enable", "payments:skill/write-spec"]);
    assert!(s.skills_dir().join("write-spec").exists());

    // Items that don't apply to you can't be enabled.
    let out = s.run(&["enable", "skill/pm-helper"]);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("not available to you"),
        "{}",
        out.stderr
    );
}

#[test]
fn required_toggle_off_is_ignored_with_warning() {
    let (s, _company, _team) = layered();
    let config = read(&s.config.join("config.toml"));
    s.write_config(&format!(
        "{config}\n[toggles]\n\"acme-company:skill/security\" = false\n"
    ));
    let out = s.ok(&["sync"]);
    assert!(
        out.stderr.contains("is required and cannot be disabled"),
        "{}",
        out.stderr
    );
    assert!(s.skills_dir().join("security").exists());
}

#[test]
fn equal_rank_conflict_exits_4_until_preferred() {
    let s = Sandbox::with_claude();
    s.write_config("[profile]\nteam = [\"payments-dev\", \"platform\"]\n");
    let a = source(&s, "alpha", "team", "payments-dev");
    a.write("skills/lint/SKILL.md", &skill("lint", "Alpha lint."));
    a.commit("a");
    let z = source(&s, "zulu", "team", "platform");
    z.write("skills/lint/SKILL.md", &skill("lint", "Zulu lint."));
    z.commit("z");
    s.ok(&["subscribe", &a.url()]);
    s.ok(&["subscribe", &z.url()]);

    let out = s.run(&["sync"]);
    assert_eq!(out.code, 4, "{}", out.stderr);
    assert!(
        out.stderr.contains("lo prefer <source:skill/lint>"),
        "{}",
        out.stderr
    );
    assert!(read(&s.skills_dir().join("lint/SKILL.md")).contains("Alpha"));

    let report = s.json(&["prefer", "zulu:skill/lint"]);
    assert_eq!(report["conflicts"], false);
    assert!(read(&s.skills_dir().join("lint/SKILL.md")).contains("Zulu"));
    assert_eq!(s.run(&["sync"]).code, 0);
    let out = s.ok(&["why", "skill/lint"]);
    assert!(
        out.stdout.contains("your `lo prefer` choice"),
        "{}",
        out.stdout
    );

    let out = s.run(&["prefer", "nope:skill/lint"]);
    assert_eq!(out.code, 1);
}

#[test]
fn priority_breaks_ties_without_conflict() {
    let s = Sandbox::with_claude();
    s.write_config("[profile]\nteam = [\"payments-dev\", \"platform\"]\n");
    let a = source(&s, "alpha", "team", "payments-dev");
    a.write("skills/lint/SKILL.md", &skill("lint", "Alpha lint."));
    a.commit("a");
    let z = source(&s, "zulu", "team", "platform");
    z.write("skills/lint/SKILL.md", &skill("lint", "Zulu lint."));
    z.commit("z");
    s.ok(&["subscribe", &a.url()]);
    s.ok(&["subscribe", &z.url(), "--priority", "5"]);
    s.ok(&["sync"]);
    assert!(read(&s.skills_dir().join("lint/SKILL.md")).contains("Zulu"));
}

#[test]
fn per_target_winners_differ_with_targets_lists() {
    let s = Sandbox::with_claude();
    s.write_config(PROFILE);
    let company = source(&s, "acme-company", "company", "acme");
    company.write("skills/x/SKILL.md", &skill("x", "Company x."));
    company.commit("c");
    let team = source(&s, "payments", "team", "payments-dev");
    team.write(
        "skills/x/SKILL.md",
        "---\ndescription: Team x, Pi only.\nloadout: { targets: [pi] }\n---\n",
    );
    team.commit("t");
    s.ok(&["subscribe", &company.url()]);
    s.ok(&["subscribe", &team.url()]);
    s.ok(&["sync"]);
    // Claude Code isn't in the team item's targets, so the company one applies.
    assert!(read(&s.skills_dir().join("x/SKILL.md")).contains("Company x."));
    let out = s.ok(&["why", "skill/x"]);
    s.insta()
        .bind(|| insta::assert_snapshot!("why_per_target", out.stdout));
}

#[test]
fn subscription_layer_and_group_place_a_source() {
    let s = Sandbox::with_claude();
    // No [profile]: a manual subscription makes you a member of its group.
    let extra = source(&s, "extra", "team", "someone-elses-team");
    extra.write("skills/x/SKILL.md", &skill("x", "Extra x."));
    extra.write(
        "skills/y/SKILL.md",
        "---\ndescription: Y.\nloadout: { layer: squad, group: checkout }\n---\n",
    );
    extra.commit("e");
    s.ok(&[
        "subscribe",
        &extra.url(),
        "--layer",
        "role",
        "--group",
        "developer",
    ]);
    s.ok(&["sync"]);
    assert!(s.skills_dir().join("x").exists());
    // y names its own group, which you are not a member of.
    assert!(!s.skills_dir().join("y").exists());
    let v = s.json(&["why", "skill/x"]);
    assert_eq!(v["candidates"][0]["layer"], "role");
    assert_eq!(v["candidates"][0]["group"], "developer");
}

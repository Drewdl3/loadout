//! `lo adopt`: skills already in an AI tool's directory (or a folder)
//! copied into a source.

mod common;

use common::{Sandbox, skill};
use loadout_git::fixture::FixtureRepo;
use serde_json::Value;

fn names(report: &Value) -> Vec<String> {
    report["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect()
}

/// A sandbox with one synced (Loadout-owned) skill, a hand-made skill with
/// a supporting file, and one whose folder name isn't kebab-case.
fn world() -> (Sandbox, FixtureRepo) {
    let s = Sandbox::with_claude();
    let team = s.source("team-skills");
    team.write(
        "skills/managed/SKILL.md",
        &skill("managed", "From Loadout."),
    );
    team.commit("init");
    s.ok(&["subscribe", &team.url()]);
    s.ok(&["sync"]);
    let mine = s.skills_dir().join("my-debugging");
    std::fs::create_dir_all(mine.join("refs")).unwrap();
    std::fs::write(
        mine.join("SKILL.md"),
        skill("my-debugging", "How I debug flaky tests."),
    )
    .unwrap();
    std::fs::write(mine.join("refs/notes.md"), "notes\n").unwrap();
    let bad = s.skills_dir().join("Bad_Name");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("SKILL.md"), skill("bad", "Bad.")).unwrap();
    (s, team)
}

#[test]
fn lists_only_skills_loadout_does_not_manage() {
    let (s, _) = world();
    let r = s.json(&["adopt"]);
    assert_eq!(names(&r), ["Bad_Name", "my-debugging"], "{r:#}");
    let bad = &r["candidates"][0];
    assert!(bad["problem"].as_str().unwrap().contains("kebab-case"));
    assert_eq!(r["candidates"][1]["found_in"], "claude-code");
    assert_eq!(
        r["candidates"][1]["description"],
        "How I debug flaky tests."
    );
    assert!(r["source"].is_null());
    // Choosing skills needs a destination; choosing one needs a terminal
    // or --skill/--all.
    let out = s.run(&["adopt", "--skill", "my-debugging"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("--into"), "{}", out.stderr);
}

#[test]
fn copies_the_whole_skill_into_a_checkout_and_never_overwrites() {
    let (s, team) = world();
    let checkout = std::path::PathBuf::from(team.url());
    let r = s.json(&[
        "adopt",
        "--dir",
        checkout.to_str().unwrap(),
        "--skill",
        "my-debugging",
    ]);
    assert_eq!(r["source"], "team-skills");
    assert_eq!(r["adopted"][0]["id"], "team-skills:skill/my-debugging");
    assert_eq!(
        r["adopted"][0]["files"],
        serde_json::json!(["SKILL.md", "refs/notes.md"])
    );
    assert_eq!(r["working_clone"], false);
    assert!(checkout.join("skills/my-debugging/refs/notes.md").is_file());
    let audit = s.run(&["audit", "--path", checkout.to_str().unwrap()]);
    assert_eq!(audit.code, 0, "{}", audit.stdout);

    // Again: skipped, exit 1, the copy is untouched. A bad name is skipped too.
    std::fs::write(checkout.join("skills/my-debugging/SKILL.md"), "edited").unwrap();
    let out = s.run(&[
        "adopt",
        "--dir",
        checkout.to_str().unwrap(),
        "--all",
        "--json",
    ]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    let r: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(r["adopted"], serde_json::json!([]));
    let reasons: Vec<&str> = r["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["reason"].as_str().unwrap())
        .collect();
    assert!(reasons[0].contains("kebab-case"), "{reasons:?}");
    assert!(
        reasons[1].contains("already has skills/my-debugging"),
        "{reasons:?}"
    );
    assert_eq!(
        std::fs::read_to_string(checkout.join("skills/my-debugging/SKILL.md")).unwrap(),
        "edited"
    );
}

#[test]
fn adopts_from_a_folder_into_a_working_clone() {
    let (s, _) = world();
    // A project's .claude/skills, given as a path.
    let project = s.root.join("project/.claude/skills");
    std::fs::create_dir_all(project.join("deploy")).unwrap();
    std::fs::write(project.join("deploy/SKILL.md"), skill("deploy", "Deploy.")).unwrap();
    let listed = s.json(&["adopt", project.to_str().unwrap()]);
    assert_eq!(names(&listed), ["deploy"]);
    assert_eq!(listed["candidates"][0]["found_in"], "path");
    // One skill folder works too.
    let one = s.json(&["adopt", project.join("deploy").to_str().unwrap()]);
    assert_eq!(names(&one), ["deploy"]);

    let r = s.json(&[
        "adopt",
        project.to_str().unwrap(),
        "--into",
        "team-skills",
        "--skill",
        "deploy",
    ]);
    assert_eq!(r["working_clone"], true);
    let path = std::path::PathBuf::from(r["adopted"][0]["path"].as_str().unwrap());
    assert!(path.join("SKILL.md").is_file());
    assert!(path.starts_with(s.data.join("work")), "{}", path.display());
}

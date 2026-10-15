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
    // `--all` leaves out what's already adopted; naming it still tries.
    assert!(
        reasons[1].contains("adopted into team-skills as skill/my-debugging"),
        "{reasons:?}"
    );
    let out = s.run(&[
        "adopt",
        "--dir",
        checkout.to_str().unwrap(),
        "--skill",
        "my-debugging",
        "--json",
    ]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    assert!(
        out.stdout.contains("already has skills/my-debugging"),
        "{}",
        out.stdout
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

#[test]
fn renames_and_edits_on_the_way_in() {
    let (s, team) = world();
    let checkout = std::path::PathBuf::from(team.url());
    let listed = s.json(&["adopt"]);
    assert_eq!(listed["candidates"][0]["suggested_name"], "bad-name");
    let edits = r#"{"bad-name": {"description": "Better. Use when testing.", "tags": ["mine"]}}"#;
    let r = s.json(&[
        "adopt",
        "--dir",
        checkout.to_str().unwrap(),
        "--skill",
        "Bad_Name=bad-name",
        "--skill",
        "my-debugging",
        "--tag",
        "imported",
        "--author",
        "Ada",
        "--mode",
        "default-off",
        "--edits",
        edits,
    ]);
    assert_eq!(r["adopted"][0]["id"], "team-skills:skill/bad-name", "{r:#}");
    assert_eq!(r["adopted"][0]["renamed_from"], "Bad_Name");
    let bad = std::fs::read_to_string(checkout.join("skills/bad-name/SKILL.md")).unwrap();
    assert!(bad.starts_with("---\nname: bad-name\n"), "{bad}");
    assert!(bad.contains("Better. Use when testing."), "{bad}");
    assert!(bad.contains(r#"tags: ["mine","imported"]"#), "{bad}");
    assert!(bad.contains(r#"author: "Ada""#), "{bad}");
    assert!(bad.contains(r#"mode: "default-off""#), "{bad}");
    let mine = std::fs::read_to_string(checkout.join("skills/my-debugging/SKILL.md")).unwrap();
    assert!(mine.contains(r#"tags: ["imported"]"#), "{mine}");
    assert!(mine.contains("How I debug flaky tests."), "{mine}");
    let audit = s.run(&["audit", "--path", checkout.to_str().unwrap()]);
    assert_eq!(audit.code, 0, "{}", audit.stdout);

    // A bad edit writes nothing.
    std::fs::remove_dir_all(checkout.join("skills/my-debugging")).unwrap();
    let out = s.run(&[
        "adopt",
        "--dir",
        checkout.to_str().unwrap(),
        "--skill",
        "my-debugging",
        "--mode",
        "sometimes",
        "--json",
    ]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    assert!(!checkout.join("skills/my-debugging").exists());
}

/// Adopts `my-debugging` into the team's repo and publishes it, with a
/// second tool (Pi) linking to the original the way `npx skills` does.
#[cfg(unix)]
fn adopted_and_published() -> (Sandbox, FixtureRepo, std::path::PathBuf) {
    let (s, team) = world();
    let pi = s.home.join(".pi/agent/skills");
    std::fs::create_dir_all(&pi).unwrap();
    std::os::unix::fs::symlink(
        "../../../.claude/skills/my-debugging",
        pi.join("my-debugging"),
    )
    .unwrap();
    let checkout = std::path::PathBuf::from(team.url());
    s.ok(&[
        "adopt",
        "--dir",
        checkout.to_str().unwrap(),
        "--skill",
        "my-debugging",
    ]);
    let r = s.json(&["adopt"]);
    let c = &r["candidates"][1];
    assert_eq!(c["name"], "my-debugging");
    assert!(
        c["already"]
            .as_str()
            .unwrap()
            .contains("adopted into team-skills"),
        "{c:#}"
    );
    team.commit("adopt my-debugging");
    (s, team, pi)
}

#[cfg(unix)]
#[test]
fn sync_moves_the_original_aside_once_the_source_ships_it() {
    let (s, _team, pi) = adopted_and_published();
    let original = s.skills_dir().join("my-debugging");

    // The update is held for review (a manual source): nothing moves yet.
    let held = s.run(&["sync", "--json"]);
    let held: Value = serde_json::from_str(&held.stdout).unwrap();
    assert!(held["handed_over"].is_null(), "{held:#}");
    assert!(original.join("SKILL.md").is_file() && !original.is_symlink());

    let r = s.json(&["approve", "team-skills"]);
    let h = &r["handed_over"][0];
    assert_eq!(h["source"], "team-skills", "{r:#}");
    assert_eq!(h["links"][0], pi.join("my-debugging").display().to_string());
    let backup = s.data.join("adopted/my-debugging");
    assert_eq!(h["backup"], backup.display().to_string());
    assert!(backup.join("refs/notes.md").is_file());
    // Loadout's copy replaced it, in both tools (Pi is installed here).
    for at in [&original, &pi.join("my-debugging")] {
        let to = std::fs::read_link(at).unwrap();
        assert!(to.starts_with(s.data.join("store")), "{}", to.display());
    }
    assert!(
        !r["warnings"].to_string().contains("not managed by Loadout"),
        "{r:#}"
    );
    // Nothing left to adopt, and the next sync has nothing to hand over.
    assert_eq!(names(&s.json(&["adopt"])), ["Bad_Name"]);
    assert!(s.json(&["sync"])["handed_over"].is_null());

    // Putting it back needs the place free.
    let out = s.run(&["adopt", "--restore", "my-debugging"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("in use"), "{}", out.stderr);
    s.ok(&["disable", "team-skills:skill/my-debugging"]);
    s.ok(&["sync"]);
    let r = s.json(&["adopt", "--restore", "my-debugging"]);
    assert_eq!(r["restored"]["path"], original.display().to_string());
    assert!(original.join("refs/notes.md").is_file() && !original.is_symlink());
    assert!(pi.join("my-debugging/SKILL.md").is_file());
    assert!(!backup.exists());
}

#[cfg(unix)]
#[test]
fn an_original_changed_since_adopting_is_left_alone() {
    let (s, _team, pi) = adopted_and_published();
    let original = s.skills_dir().join("my-debugging");
    std::fs::write(original.join("refs/notes.md"), "more notes\n").unwrap();
    s.run(&["sync"]);
    let r = s.json(&["approve", "team-skills"]);
    assert!(r["handed_over"].is_null(), "{r:#}");
    let warnings = r["warnings"].to_string();
    assert!(
        warnings.contains("changed since you adopted it"),
        "{warnings}"
    );
    assert!(!original.is_symlink());
    assert!(pi.join("my-debugging").is_symlink());
}

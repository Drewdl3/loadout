//! End-to-end: subscribe → sync → skills appear in (fake) Claude Code.

mod common;

use common::{Sandbox, skill};

fn read(p: &std::path::Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

#[test]
fn skill_from_source_appears_in_claude_code() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "Write a spec."),
    )
    .write("skills/write-spec/templates/spec.md", "# Spec\n");
    repo.commit("initial");

    let out = s.ok(&["subscribe", &repo.url()]);
    s.insta()
        .bind(|| insta::assert_snapshot!("subscribe", out.stdout));

    let out = s.ok(&["sync"]);
    s.insta()
        .bind(|| insta::assert_snapshot!("first_sync", out.stdout));
    assert_eq!(out.stderr, "");

    let installed = s.skills_dir().join("write-spec");
    assert!(installed.join("SKILL.md").is_file());
    assert_eq!(read(&installed.join("templates/spec.md")), "# Spec\n");
    assert!(
        std::fs::symlink_metadata(&installed)
            .unwrap()
            .file_type()
            .is_symlink()
            || cfg!(windows),
        "installed via link"
    );
}

#[test]
fn resync_is_idempotent() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write("skills/a/SKILL.md", &skill("a", "A."))
        .write("skills/b/SKILL.md", &skill("b", "B."));
    repo.commit("initial");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);
    let owned1 = read(&s.data.join("owned.json"));
    let resolved1 = read(&s.data.join("resolved.json"));

    let out = s.ok(&["sync"]);
    s.insta()
        .bind(|| insta::assert_snapshot!("resync", out.stdout));
    assert_eq!(read(&s.data.join("owned.json")), owned1);
    assert_eq!(read(&s.data.join("resolved.json")), resolved1);
}

#[test]
fn unowned_files_are_never_touched() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "Theirs."),
    )
    .write("skills/other/SKILL.md", &skill("other", "Other."));
    repo.commit("initial");

    // The user already has their own skill with the same name, plus another one.
    let mine = s.skills_dir().join("write-spec");
    std::fs::create_dir_all(&mine).unwrap();
    std::fs::write(mine.join("SKILL.md"), "my own skill").unwrap();
    let personal = s.skills_dir().join("personal");
    std::fs::create_dir_all(&personal).unwrap();
    std::fs::write(personal.join("SKILL.md"), "personal").unwrap();

    s.ok(&["subscribe", &repo.url()]);
    let out = s.ok(&["sync"]);
    s.insta().bind(|| {
        insta::assert_snapshot!("collision_stdout", out.stdout);
        insta::assert_snapshot!("collision_stderr", out.stderr);
    });

    assert_eq!(read(&mine.join("SKILL.md")), "my own skill");
    assert_eq!(read(&personal.join("SKILL.md")), "personal");
    assert!(s.skills_dir().join("other/SKILL.md").is_file());

    // Removing everything upstream still leaves the user's files alone.
    s.ok(&["unsubscribe", &repo.url()]);
    s.ok(&["sync"]);
    assert_eq!(read(&mine.join("SKILL.md")), "my own skill");
    assert_eq!(read(&personal.join("SKILL.md")), "personal");
    assert!(!s.skills_dir().join("other").exists());

    let doctor = s.json(&["doctor"]);
    assert!(
        doctor["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["check"] != "collision"),
        "no collision once nothing is wanted: {doctor:#}"
    );
}

#[test]
fn doctor_reports_collisions() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "Theirs."),
    );
    repo.commit("initial");
    std::fs::create_dir_all(s.skills_dir().join("write-spec")).unwrap();
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);

    let out = s.ok(&["doctor"]);
    let filtered = out
        .stdout
        .lines()
        .filter(|l| !l.contains("git version"))
        .collect::<Vec<_>>()
        .join("\n");
    s.insta()
        .bind(|| insta::assert_snapshot!("doctor_collision", filtered));
}

#[test]
fn upstream_changes_propagate() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write("skills/a/SKILL.md", &skill("a", "v1"))
        .write("skills/gone/SKILL.md", &skill("gone", "bye"));
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);
    assert!(s.skills_dir().join("gone").exists());

    repo.write("skills/a/SKILL.md", &skill("a", "v2"))
        .remove("skills/gone")
        .write("skills/new/SKILL.md", &skill("new", "hi"));
    repo.commit("v2");
    // Updates to a manual source wait for review; --yes approves them.
    let report = s.json(&["sync", "--yes"]);
    let t = &report["targets"][0];
    assert_eq!(t["id"], "claude-code");
    assert_eq!(t["added"], serde_json::json!(["skill/new"]));
    assert_eq!(t["removed"], serde_json::json!(["skill/gone"]));
    assert!(read(&s.skills_dir().join("a/SKILL.md")).contains("v2"));
    assert!(!s.skills_dir().join("gone").exists());
}

#[test]
fn copy_mode_installs_copies_and_updates_them() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write("skills/a/SKILL.md", &skill("a", "v1"));
    repo.commit("v1");
    s.write_config("[targets]\nlink_mode = \"copy\"\n");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);
    let a = s.skills_dir().join("a");
    assert!(
        !std::fs::symlink_metadata(&a)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(read(&a.join("SKILL.md")).contains("v1"));

    repo.write("skills/a/SKILL.md", &skill("a", "v2"));
    repo.commit("v2");
    let report = s.json(&["sync", "--yes"]);
    assert_eq!(
        report["targets"][0]["updated"],
        serde_json::json!(["skill/a"])
    );
    assert!(read(&a.join("SKILL.md")).contains("v2"));
}

#[test]
fn default_off_items_need_a_toggle() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write(
        "skills/opt-in/SKILL.md",
        "---\ndescription: Opt in.\nloadout: { mode: default-off }\n---\n",
    );
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);
    assert!(!s.skills_dir().join("opt-in").exists());

    let config = read(&s.config.join("config.toml"));
    s.write_config(&format!(
        "{config}\n[toggles]\n\"team-skills:skill/opt-in\" = true\n"
    ));
    s.ok(&["sync"]);
    assert!(s.skills_dir().join("opt-in/SKILL.md").is_file());
}

#[test]
fn fetch_failure_uses_cached_checkout() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write("skills/a/SKILL.md", &skill("a", "A."));
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);

    std::fs::remove_dir_all(&repo.path).unwrap();
    let out = s.ok(&["sync"]);
    assert!(out.stdout.contains("(cached)"), "{}", out.stdout);
    assert!(out.stderr.contains("fetch failed, using cached checkout"));
    assert!(s.skills_dir().join("a/SKILL.md").is_file());
}

#[test]
fn unreachable_new_source_fails_without_changes() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write("skills/a/SKILL.md", &skill("a", "A."));
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);

    // Subscribed while it existed, gone by the next sync.
    let missing = s.source("missing");
    missing.commit("v1");
    s.ok(&["subscribe", &missing.url()]);
    std::fs::remove_dir_all(missing.url()).unwrap();
    let out = s.run(&["sync"]);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("could not load 1 source(s)"),
        "{}",
        out.stderr
    );
    assert!(s.skills_dir().join("a/SKILL.md").is_file());
}

#[test]
fn no_detected_tools_warns() {
    let s = Sandbox::new();
    let out = s.ok(&["sync"]);
    assert!(
        out.stderr.contains("no AI tools detected"),
        "{}",
        out.stderr
    );
    assert!(out.stdout.contains("No sources subscribed"));
}

#[test]
fn item_target_include_list_is_respected() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write(
        "skills/pi-only/SKILL.md",
        "---\ndescription: Pi only.\nloadout: { targets: [pi] }\n---\n",
    )
    .write("skills/everywhere/SKILL.md", &skill("everywhere", "All."));
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);
    assert!(!s.skills_dir().join("pi-only").exists());
    assert!(s.skills_dir().join("everywhere").exists());
}

#[test]
fn disabling_a_target_removes_its_entries() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write("skills/a/SKILL.md", &skill("a", "A."));
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);
    assert!(s.skills_dir().join("a").exists());

    let config = read(&s.config.join("config.toml"));
    s.write_config(&format!("{config}\n[targets]\nenabled = []\n"));
    let report = s.json(&["sync"]);
    assert_eq!(
        report["targets"][0]["removed"],
        serde_json::json!(["skill/a"])
    );
    assert!(!s.skills_dir().join("a").exists());
}

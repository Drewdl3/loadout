//! Crash safety: a sync killed at any point during apply is
//! repaired by the next sync. `LOADOUT_TEST_CRASH=<point>` (test-only
//! hook) aborts the process there, like a kill.

mod common;

use std::path::Path;

use common::{Sandbox, skill};

fn mcp(name: &str, version: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {version}\ncommand: acme-{name}\nargs: [\"{version}\"]\n---\n"
    )
}

/// A sandbox synced at v1, with v2 committed upstream.
fn setup(link_mode: &str) -> Sandbox {
    let s = Sandbox::with_claude();
    s.write_config(&format!("[targets]\nlink_mode = \"{link_mode}\"\n"));
    std::fs::write(s.home.join(".claude.json"), "{\n  \"mcpServers\": {}\n}\n").unwrap();
    let repo = s.source("team-skills");
    repo.write("skills/a/SKILL.md", &skill("a", "A v1."))
        .write("skills/b/SKILL.md", &skill("b", "B v1."))
        .write("mcp/plain.md", &mcp("plain", "v1"));
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);
    repo.write("skills/a/SKILL.md", &skill("a", "A v2."))
        .remove("skills/b")
        .write("skills/c/SKILL.md", &skill("c", "C v2."))
        .write("mcp/plain.md", &mcp("plain", "v2"))
        .write("mcp/other.md", &mcp("other", "v2"));
    repo.commit("v2");
    s
}

fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// Everything a finished v2 sync should leave behind.
fn assert_consistent(s: &Sandbox, point: &str) {
    let skills = s.skills_dir();
    assert_eq!(listing(&skills), ["a", "c"], "{point}: skills dir");
    let a = std::fs::read_to_string(skills.join("a/SKILL.md")).unwrap();
    assert!(a.contains("A v2."), "{point}: a is v2");
    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(s.home.join(".claude.json")).unwrap())
            .unwrap();
    assert_eq!(cfg["mcpServers"]["plain"]["args"][0], "v2", "{point}");
    assert_eq!(cfg["mcpServers"]["other"]["args"][0], "v2", "{point}");

    let owned: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(s.data.join("owned.json")).unwrap()).unwrap();
    let dests: Vec<String> = owned["targets"]["claude-code"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["item"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(dests, ["skill/a", "skill/c"], "{point}: owned links");
    assert!(
        !owned.to_string().contains("pending"),
        "{point}: no pending placements left"
    );
    assert_eq!(
        owned["mcp"]["claude-code"]["servers"],
        serde_json::json!(["other", "plain"]),
        "{point}: owned MCP servers"
    );
    assert!(!s.data.join("store.new").exists() && !s.data.join("store.old").exists());

    let doctor = s.run(&["doctor"]);
    assert_eq!(doctor.code, 0, "{point}: doctor\n{}", doctor.stdout);
    assert!(
        !doctor.stdout.contains("warn"),
        "{point}: {}",
        doctor.stdout
    );
    let status = s.run(&["status"]);
    assert_eq!(status.code, 0, "{point}: {}", status.stdout);
}

fn crash_then_recover(link_mode: &str, point: &str) {
    let s = setup(link_mode);
    let out = s.run_env(&["sync", "--yes"], &[("LOADOUT_TEST_CRASH", point)]);
    assert_ne!(out.code, 0, "{point}: the sync should have been killed");
    assert!(
        out.stderr
            .contains(&format!("aborting at {}", point.split(':').next().unwrap())),
        "{point}: crash point not reached\n{}",
        out.stderr
    );
    let out = s.run(&["sync", "--yes"]);
    assert_eq!(out.code, 0, "{point}: recovery sync\n{}", out.stderr);
    assert!(
        !out.stderr.contains("not managed by Loadout"),
        "{point}: our own entries look foreign after the crash\n{}",
        out.stderr
    );
    assert_consistent(&s, point);
}

const POINTS: &[&str] = &[
    "after-store",
    "after-intent",
    "link:1",
    "link:2",
    "link:3",
    "target",
    "before-owned",
    "before-resolved",
    "before-lock",
    "before-state",
];

#[test]
fn copy_mode_recovers_from_a_crash_anywhere() {
    for point in POINTS {
        crash_then_recover("copy", point);
    }
}

#[test]
fn symlink_mode_recovers_from_a_crash_anywhere() {
    for point in ["after-intent", "link:1", "link:2", "before-owned"] {
        crash_then_recover("symlink", point);
    }
}

#[test]
fn a_clean_sync_matches_the_reference() {
    let s = setup("copy");
    s.ok(&["sync", "--yes"]);
    assert_consistent(&s, "no crash");
}

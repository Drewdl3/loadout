//! subscribe / unsubscribe / list / doctor behavior and JSON output.

mod common;

use common::{Sandbox, skill};

#[test]
fn subscribe_preserves_config_comments_and_dedupes() {
    let s = Sandbox::new();
    s.write_config("# my settings\n[targets]\nlink_mode = \"symlink\" # keep\n");
    let first = s.json(&[
        "subscribe",
        "https://example.com/acme/skills.git",
        "--priority",
        "5",
    ]);
    assert_eq!(first["changed"], true);
    let again = s.json(&["subscribe", "https://example.com/acme/skills"]);
    assert_eq!(again["changed"], false);

    let text = std::fs::read_to_string(s.config.join("config.toml")).unwrap();
    assert!(text.contains("# my settings"));
    assert!(text.contains("# keep"));
    assert!(text.contains("priority = 5"));
    assert!(s.config.join("config.toml.loadout.bak").is_file());
}

#[test]
fn unsubscribe_unknown_is_an_error() {
    let s = Sandbox::new();
    let out = s.run(&["unsubscribe", "https://example.com/nope"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("not subscribed"), "{}", out.stderr);

    let out = s.run(&["unsubscribe", "https://example.com/nope", "--json"]);
    assert_eq!(out.code, 1);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert!(v["error"].as_str().unwrap().contains("not subscribed"));
}

#[test]
fn invalid_config_is_reported() {
    let s = Sandbox::new();
    s.write_config("chartr = \"typo\"\n");
    let out = s.run(&["sync"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("config.toml"), "{}", out.stderr);
}

#[test]
fn list_before_and_after_sync() {
    let s = Sandbox::with_claude();
    let out = s.ok(&["list"]);
    assert!(out.stdout.contains("Nothing synced yet"));

    let repo = s.source("team-skills");
    repo.write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "Write a spec."),
    )
    .write(
        "skills/opt-in/SKILL.md",
        "---\ndescription: Optional helper.\nloadout: { mode: default-off }\n---\n",
    );
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);

    let out = s.ok(&["list"]);
    s.insta()
        .bind(|| insta::assert_snapshot!("list", out.stdout));
    let json = s.json(&["list"]);
    s.insta()
        .bind(|| insta::assert_json_snapshot!("list_json", json));

    let enabled = s.json(&["list", "--enabled"]);
    assert_eq!(enabled["items"].as_array().unwrap().len(), 1);
    let disabled = s.json(&["list", "--disabled"]);
    assert_eq!(disabled["items"][0]["name"], "opt-in");
    let mcp = s.json(&["list", "--kind", "mcp"]);
    assert!(mcp["items"].as_array().unwrap().is_empty());
    // --target lists what is installed there: enabled items only.
    let cc = s.json(&["list", "--target", "claude-code"]);
    assert_eq!(cc["items"].as_array().unwrap().len(), 1);
    assert_eq!(s.run(&["list", "--target", "nope"]).code, 1);
}

#[test]
fn doctor_on_fresh_machine() {
    let s = Sandbox::new();
    let v = s.json(&["doctor"]);
    let checks = v["checks"].as_array().unwrap();
    let find = |id: &str| checks.iter().find(|c| c["check"] == id).unwrap();
    assert_eq!(find("git")["status"], "ok");
    assert_eq!(find("config")["status"], "ok");
    assert_eq!(find("sync")["status"], "warn");
}

#[test]
fn doctor_flags_broken_and_removed_entries() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write("skills/a/SKILL.md", &skill("a", "A."))
        .write("skills/b/SKILL.md", &skill("b", "B."));
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);

    // User deletes one entry; the store vanishes under the other.
    loadout_remove(&s.skills_dir().join("a"));
    std::fs::remove_dir_all(s.data.join("store")).unwrap();

    let out = s.run(&["doctor", "--json"]);
    assert_eq!(out.code, 1);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let ids: Vec<&str> = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["status"] != "ok")
        .map(|c| c["check"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["missing", "broken-link", "store"]);

    // A sync repairs everything.
    s.ok(&["sync"]);
    s.ok(&["doctor"]);
}

/// Removes a link (symlink or junction) without following it.
fn loadout_remove(p: &std::path::Path) {
    if std::fs::remove_file(p).is_err() {
        std::fs::remove_dir(p).unwrap();
    }
}

#[test]
fn quiet_suppresses_human_output() {
    let s = Sandbox::new();
    let out = s.ok(&["subscribe", "https://example.com/x", "--quiet"]);
    assert_eq!(out.stdout, "");
}

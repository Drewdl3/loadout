//! `lo info` and `lo targets`.

mod common;

use common::{Sandbox, skill};

fn synced() -> Sandbox {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write(
        "LOADOUT.md",
        "---\nloadout: 1\nname: team-skills\nlayer: team\ngroup: payments-dev\nowners: [\"@acme/payments\"]\ndescription: Payments skills.\n---\n# Team skills\n\nRead me.\n",
    )
    .write("skills/write-spec/SKILL.md", &skill("write-spec", "Specs."))
    .write("agents/reviewer.md", "---\ndescription: Reviews.\n---\nReview it.\n");
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);
    s
}

#[test]
fn info_renders_sources_and_items() {
    let s = synced();
    let src = s.ok(&["info", "team-skills"]);
    s.insta()
        .bind(|| insta::assert_snapshot!("info_source", src.stdout));
    let j = s.json(&["info", "team-skills"]);
    assert_eq!(j["type"], "source");
    assert_eq!(j["owners"], serde_json::json!(["@acme/payments"]));
    assert_eq!(
        j["items"],
        serde_json::json!(["agent/reviewer", "skill/write-spec"])
    );

    let item = s.json(&["info", "skill/write-spec"]);
    assert_eq!(item["type"], "item");
    assert_eq!(item["id"], "team-skills:skill/write-spec");
    assert!(
        item["text"]
            .as_str()
            .unwrap()
            .contains("Do the write-spec thing.")
    );
    let agent = s.json(&["info", "team-skills:agent/reviewer"]);
    assert!(agent["text"].as_str().unwrap().contains("Review it."));

    assert_ne!(s.run(&["info", "nope"]).code, 0);
    assert_ne!(s.run(&["info", "skill/nope"]).code, 0);
}

#[test]
fn targets_list_enable_disable_mode() {
    let s = synced();
    let j = s.json(&["targets"]);
    assert_eq!(j["auto_detect"], true);
    let row = |j: &serde_json::Value, id: &str| {
        j["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(row(&j, "claude-code")["enabled"], true);
    assert_eq!(row(&j, "claude-code")["detected"], true);
    assert_eq!(row(&j, "pi")["enabled"], false);
    assert_eq!(row(&j, "pi")["paths"][0]["path"], "~/.pi/agent/skills");

    // Enable a tool that isn't installed yet: its items appear there.
    let j = s.json(&["targets", "enable", "pi"]);
    assert_eq!(j["auto_detect"], false);
    assert_eq!(row(&j, "pi")["enabled"], true);
    assert!(
        s.home
            .join(".pi/agent/skills/write-spec/SKILL.md")
            .is_file()
    );
    let config = std::fs::read_to_string(s.config.join("config.toml")).unwrap();
    assert!(
        config.contains("enabled = [\"claude-code\", \"pi\"]"),
        "{config}"
    );

    // Copy mode for Claude Code.
    s.json(&["targets", "mode", "claude-code", "copy"]);
    let entry = s.skills_dir().join("write-spec");
    assert!(
        !std::fs::symlink_metadata(&entry)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(entry.join("SKILL.md").is_file());
    let config = std::fs::read_to_string(s.config.join("config.toml")).unwrap();
    assert!(config.contains("claude-code = \"copy\""), "{config}");

    // Disable: our entries go.
    s.json(&["targets", "disable", "claude-code"]);
    assert!(!entry.exists());
    assert!(s.home.join(".pi/agent/skills/write-spec").exists());

    assert_ne!(s.run(&["targets", "enable", "vim"]).code, 0);
}

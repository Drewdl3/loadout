//! Claude Code plugins: a generated local marketplace,
//! registered in `~/.claude/settings.json` without touching other keys.

mod common;

use common::Sandbox;

const SETTINGS: &str =
    "{\n  \"theme\": \"dark\",\n  \"enabledPlugins\": {\n    \"mine@official\": true\n  }\n}\n";

fn read_json(p: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn plugins_are_published_and_registered() {
    let s = Sandbox::with_claude();
    std::fs::create_dir_all(s.home.join(".pi")).unwrap();
    let settings = s.home.join(".claude/settings.json");
    std::fs::write(&settings, SETTINGS).unwrap();

    let repo = s.source("team-skills");
    repo.write(
        "plugins/review-kit/PLUGIN.md",
        "---\nname: review-kit\ndescription: Review helpers.\nversion: 1.2.0\n---\n# Review kit\n",
    )
    .write(
        "plugins/review-kit/commands/review.md",
        "---\ndescription: Review.\n---\nReview it.\n",
    )
    .write("plugins/not-a-plugin/README.md", "no manifest");
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    let out = s.run(&["sync", "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let r: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let cc = r["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "claude-code")
        .unwrap();
    assert!(
        cc["added"]
            .as_array()
            .unwrap()
            .contains(&"plugin/review-kit".into()),
        "{cc}"
    );
    let warnings = r["warnings"].to_string();
    assert!(
        warnings.contains("plugins/not-a-plugin: no PLUGIN.md"),
        "{warnings}"
    );
    assert!(
        warnings.contains("installed only into Claude Code; pi don't support plugins"),
        "{warnings}"
    );

    let market = s.data.join("claude-marketplace");
    let m = read_json(&market.join(".claude-plugin/marketplace.json"));
    assert_eq!(m["plugins"][0]["name"], "review-kit");
    assert_eq!(m["plugins"][0]["version"], "1.2.0");
    assert!(market.join("review-kit/commands/review.md").is_file());
    assert_eq!(
        read_json(&market.join("review-kit/.claude-plugin/plugin.json"))["description"],
        "Review helpers."
    );

    let st = read_json(&settings);
    assert_eq!(st["theme"], "dark");
    assert_eq!(st["enabledPlugins"]["mine@official"], true);
    assert_eq!(st["enabledPlugins"]["review-kit@loadout-sync"], true);
    let src = &st["extraKnownMarketplaces"]["loadout-sync"]["source"];
    assert_eq!(src["source"], "directory");
    assert_eq!(
        std::path::Path::new(src["path"].as_str().unwrap()),
        market.as_path()
    );
    assert_eq!(
        std::fs::read_to_string(s.home.join(".claude/settings.json.loadout.bak")).unwrap(),
        SETTINGS
    );

    // Re-sync is a no-op; disabling the plugin unregisters only our keys.
    let before = std::fs::read_to_string(&settings).unwrap();
    s.ok(&["sync"]);
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), before);
    s.ok(&["disable", "plugin/review-kit"]);
    let st = read_json(&settings);
    assert_eq!(
        st["enabledPlugins"],
        serde_json::json!({"mine@official": true})
    );
    assert_eq!(st["extraKnownMarketplaces"], serde_json::json!({}));
    assert_eq!(st["theme"], "dark");
    assert!(!market.join("review-kit").exists());
}

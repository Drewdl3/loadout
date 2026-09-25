//! Harness-agnostic use: the IBM Bob target, and a tool
//! described only by a user target file.

mod common;

use common::{Sandbox, skill};
use serde_json::Value;

fn read_json(p: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn bob_gets_skills_rules_commands_and_mcp() {
    let s = Sandbox::new();
    std::fs::create_dir_all(s.home.join(".bob")).unwrap();
    // The user's own MCP server survives.
    std::fs::write(
        s.home.join(".bob/mcp_settings.json"),
        "{\"mcpServers\": {\"mine\": {\"command\": \"my-mcp\"}}}\n",
    )
    .unwrap();
    let repo = s.source("team-skills");
    repo.write("skills/write-spec/SKILL.md", &skill("write-spec", "Specs."))
        .write(
            "extras/rules/style.md",
            "---\ndescription: Style.\n---\nUse tabs.\n",
        )
        .write(
            "extras/commands/deploy.md",
            "---\ndescription: Deploy.\n---\nGo.\n",
        )
        .write(
            "mcp/jira.md",
            "---\nname: jira\ncommand: jira-mcp\nenv: { JIRA_TOKEN: \"secret://jira/token\" }\n---\n",
        )
        .write(
            "mcp/docs.md",
            "---\nname: docs\ntransport: http\nurl: https://docs.example.com/mcp\n---\n",
        );
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    let r = s.json(&["sync"]);
    assert_eq!(r["targets"][0]["id"], "bob", "{r:#}");

    let h = &s.home;
    assert!(h.join(".bob/skills/write-spec/SKILL.md").is_file());
    assert!(h.join(".bob/rules/style.md").is_file());
    assert!(h.join(".bob/commands/deploy.md").is_file());
    let mcp = read_json(&h.join(".bob/mcp_settings.json"));
    let servers = &mcp["mcpServers"];
    assert_eq!(servers["mine"]["command"], "my-mcp");
    // Secrets go through the launch wrapper, never into the file.
    assert_eq!(servers["jira"]["args"][0], "mcp-run");
    assert!(!mcp.to_string().contains("secret://"));
    assert_eq!(servers["docs"]["httpURL"], "https://docs.example.com/mcp");
    assert!(servers["docs"].get("url").is_none());
}

#[test]
fn any_tool_from_a_target_file() {
    let s = Sandbox::new();
    std::fs::create_dir_all(s.home.join(".my-agent")).unwrap();
    std::fs::create_dir_all(s.config.join("targets")).unwrap();
    std::fs::write(
        s.config.join("targets/my-agent.toml"),
        "id = \"my-agent\"\ndisplay = \"My Agent\"\ndetect = [\"~/.my-agent\"]\n\n[skills]\npath = \"~/.my-agent/skills\"\nlayout = \"dir-per-item\"\n\n[mcp]\nrenderer = \"cursor-json\"\npath = \"~/.my-agent/mcp.json\"\nkey = \"mcpServers\"\n",
    )
    .unwrap();
    let repo = s.source("team-skills");
    repo.write("skills/write-spec/SKILL.md", &skill("write-spec", "Specs."))
        .write("mcp/docs.md", "---\nname: docs\ncommand: docs-mcp\n---\n");
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    let r = s.json(&["sync"]);
    assert_eq!(r["targets"][0]["id"], "my-agent", "{r:#}");
    assert!(
        s.home
            .join(".my-agent/skills/write-spec/SKILL.md")
            .is_file()
    );
    let mcp = read_json(&s.home.join(".my-agent/mcp.json"));
    assert_eq!(mcp["mcpServers"]["docs"]["command"], "docs-mcp");
    let targets = s.json(&["targets"]);
    assert!(targets.to_string().contains("My Agent"));
}

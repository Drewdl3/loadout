//! All v1 targets: one source installed into Claude Code, Pi,
//! Codex, Cursor and OpenCode at once, each in its own format.

mod common;

use common::{Sandbox, skill};

fn read(p: &std::path::Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn one_source_installs_into_every_detected_tool() {
    let s = Sandbox::with_claude();
    for d in [".pi", ".codex", ".cursor", ".config/opencode"] {
        std::fs::create_dir_all(s.home.join(d)).unwrap();
    }
    // Existing user settings in each tool's config survive.
    std::fs::write(
        s.home.join(".codex/config.toml"),
        "model = \"gpt-5\" # mine\n",
    )
    .unwrap();
    std::fs::write(
        s.home.join(".config/opencode/opencode.json"),
        "{\n  \"theme\": \"dark\"\n}\n",
    )
    .unwrap();

    let repo = s.source("team-skills");
    repo.write("skills/write-spec/SKILL.md", &skill("write-spec", "Specs."))
        .write(
            "agents/reviewer.md",
            "---\nname: reviewer\ndescription: Reviews.\n---\nReview.\n",
        )
        .write(
            "extras/commands/deploy.md",
            "---\ndescription: Deploy.\n---\nGo.\n",
        )
        .write(
            "extras/prompts/triage.md",
            "---\ndescription: Triage.\n---\nTriage $1.\n",
        )
        .write(
            "mcp/docs.md",
            "---\nname: docs\ncommand: acme-docs-mcp\nargs: [\"--stdio\"]\n---\n",
        );
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    let r = s.json(&["sync"]);
    let ids: Vec<&str> = r["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["claude-code", "codex", "cursor", "opencode", "pi"]);

    let h = &s.home;
    // Skills: each tool's own directory.
    for dir in [
        ".claude/skills",
        ".agents/skills",
        ".cursor/skills",
        ".config/opencode/skills",
        ".pi/agent/skills",
    ] {
        assert!(h.join(dir).join("write-spec/SKILL.md").is_file(), "{dir}");
    }
    // Subagents where supported.
    for dir in [
        ".claude/agents",
        ".cursor/agents",
        ".config/opencode/agents",
    ] {
        assert!(h.join(dir).join("reviewer.md").is_file(), "{dir}");
    }
    assert!(!h.join(".pi/agent/agents").exists());
    // Commands vs Pi prompt templates.
    assert!(h.join(".claude/commands/deploy.md").is_file());
    assert!(h.join(".config/opencode/commands/deploy.md").is_file());
    assert!(h.join(".pi/agent/prompts/triage.md").is_file());
    assert!(!h.join(".claude/commands/triage.md").exists());

    // MCP in each native format.
    let mut configs = String::new();
    for f in [
        ".claude.json",
        ".codex/config.toml",
        ".cursor/mcp.json",
        ".config/opencode/opencode.json",
        ".pi/agent/mcp.json",
    ] {
        configs.push_str(&format!("== {f}\n{}\n", read(&h.join(f))));
    }
    s.insta()
        .bind(|| insta::assert_snapshot!("mcp_configs", configs));

    // Disabling one target removes only what we put there.
    let cfg = read(&s.config.join("config.toml"));
    s.write_config(&format!(
        "{cfg}\n[targets]\nenabled = [\"claude-code\", \"cursor\", \"opencode\", \"pi\"]\n"
    ));
    s.ok(&["sync"]);
    assert!(!h.join(".agents/skills/write-spec").exists());
    assert_eq!(
        read(&h.join(".codex/config.toml")),
        "model = \"gpt-5\" # mine\n"
    );
    assert!(h.join(".cursor/skills/write-spec").exists());
}

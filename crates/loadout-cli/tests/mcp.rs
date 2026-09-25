//! End-to-end: MCP servers from a source are merged into (fake) Claude
//! Code's `~/.claude.json` without secrets or disturbing user entries.

mod common;

use common::Sandbox;

const JIRA: &str = r#"---
name: jira
description: Jira for Acme tickets.
loadout: { mode: required }
command: npx
args: ["-y", "@acme/jira-mcp"]
env:
  JIRA_BASE_URL: "https://acme.example.com"
  JIRA_TOKEN: "secret://jira/token"
---
Connects the agent to Jira.
"#;

const DOCS: &str = r#"---
name: docs
description: Acme docs search.
url: https://mcp.example.com/docs
headers:
  Authorization: "Bearer secret://docs/token"
  X-Team: payments
---
"#;

const PLAIN: &str = r#"---
name: plain
description: No secrets.
command: acme-plain-mcp
args: ["--stdio"]
---
"#;

const CLAUDE_JSON: &str = r#"{
  "numStartups": 7,
  "mcpServers": {
    "mine": {
      "type": "stdio",
      "command": "my-server"
    }
  },
  "projects": {}
}
"#;

fn read(p: &std::path::Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

fn setup() -> (Sandbox, loadout_git::fixture::FixtureRepo) {
    let s = Sandbox::with_claude();
    std::fs::write(s.home.join(".claude.json"), CLAUDE_JSON).unwrap();
    let repo = s.source("team-skills");
    repo.write("mcp/jira.md", JIRA)
        .write("mcp/docs.md", DOCS)
        .write("mcp/plain.md", PLAIN);
    repo.commit("mcp servers");
    s.ok(&["subscribe", &repo.url()]);
    (s, repo)
}

/// Replaces the `lo` binary path (as written into configs) with `[LOADOUT]`.
fn hide_bin(text: &str) -> String {
    let bin = loadout_cli::engine::loadout_bin_for(&assert_cmd::cargo::cargo_bin("lo"));
    text.replace(&bin.replace('\\', "\\\\"), "[LOADOUT]")
        .replace(&bin, "[LOADOUT]")
}

#[test]
fn servers_are_merged_into_claude_json_without_secrets() {
    let (s, _repo) = setup();
    let out = s.ok(&["sync"]);
    assert_eq!(out.stderr, "", "{}", out.stderr);

    let cfg = read(&s.home.join(".claude.json"));
    let shown = hide_bin(&cfg);
    s.insta()
        .bind(|| insta::assert_snapshot!("claude_json", shown));
    assert!(
        !cfg.contains("secret://"),
        "references stay out of the config"
    );

    // Backup of the pre-write version.
    assert_eq!(read(&s.home.join(".claude.json.loadout.bak")), CLAUDE_JSON);

    // Ownership is tracked.
    let owned: serde_json::Value = serde_json::from_str(&read(&s.data.join("owned.json"))).unwrap();
    assert_eq!(
        owned["mcp"]["claude-code"]["servers"],
        serde_json::json!(["docs", "jira", "plain"])
    );
}

#[test]
fn resync_is_a_no_op_and_removal_touches_only_ours() {
    let (s, repo) = setup();
    s.ok(&["sync"]);
    let first = read(&s.home.join(".claude.json"));
    let out = s.ok(&["sync"]);
    assert_eq!(read(&s.home.join(".claude.json")), first);
    assert!(out.stdout.contains("unchanged"), "{}", out.stdout);

    // The user edits their own server; upstream drops two of ours.
    let edited = first.replace("\"my-server\"", "\"my-server-v2\"");
    std::fs::write(s.home.join(".claude.json"), &edited).unwrap();
    repo.remove("mcp/docs.md").remove("mcp/plain.md");
    repo.commit("drop docs and plain");
    s.ok(&["sync", "--yes"]);
    let cfg: serde_json::Value = serde_json::from_str(&read(&s.home.join(".claude.json"))).unwrap();
    let names: Vec<&str> = cfg["mcpServers"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(names, ["mine", "jira"]);
    assert_eq!(cfg["mcpServers"]["mine"]["command"], "my-server-v2");
    assert_eq!(cfg["numStartups"], 7);
}

#[test]
fn unmanaged_server_with_same_name_is_left_alone() {
    let s = Sandbox::with_claude();
    let theirs = CLAUDE_JSON.replace("\"mine\"", "\"plain\"");
    std::fs::write(s.home.join(".claude.json"), &theirs).unwrap();
    let repo = s.source("team-skills");
    repo.write("mcp/plain.md", PLAIN);
    repo.commit("plain");
    s.ok(&["subscribe", &repo.url()]);
    let out = s.ok(&["sync"]);
    assert!(
        out.stderr
            .contains("already has an MCP server named \"plain\""),
        "{}",
        out.stderr
    );
    assert_eq!(read(&s.home.join(".claude.json")), theirs);
}

#[test]
fn disabled_target_loses_only_our_servers() {
    let (s, _repo) = setup();
    s.ok(&["sync"]);
    s.write_config(&format!(
        "{}\n[targets]\nenabled = []\n",
        read(&s.config.join("config.toml"))
    ));
    s.ok(&["sync"]);
    assert_eq!(read(&s.home.join(".claude.json")), CLAUDE_JSON);
    let owned: serde_json::Value = serde_json::from_str(&read(&s.data.join("owned.json"))).unwrap();
    assert!(owned.get("mcp").is_none(), "{owned}");
}

#[test]
fn http_secret_needs_opt_in_without_headers_helper() {
    // A user-defined target like Claude Code but without headersHelper.
    let (s, _repo) = setup();
    let targets = s.config.join("targets");
    std::fs::create_dir_all(&targets).unwrap();
    std::fs::write(
        targets.join("cursor-like.toml"),
        "id = \"cursor-like\"\ndisplay = \"Cursor-like\"\n[mcp]\nrenderer = \"cursor-json\"\npath = \"~/.cursor-like/mcp.json\"\n",
    )
    .unwrap();
    let base = read(&s.config.join("config.toml"));
    s.write_config(&format!("{base}\n[targets]\nenabled = [\"cursor-like\"]\n"));
    let out = s.ok(&["sync"]);
    assert!(
        out.stderr
            .contains("skipped mcp/docs: header Authorization contains secret references"),
        "{}",
        out.stderr
    );
    let cfg = read(&s.home.join(".cursor-like/mcp.json"));
    assert!(!cfg.contains("\"docs\""));

    // Opt in: the value is written, and the file is private.
    s.write_config(&format!(
        "{base}\n[targets]\nenabled = [\"cursor-like\"]\n[secrets]\nallow_secrets_on_disk = true\n"
    ));
    let out = s.run_env(&["sync"], &[("DOCS_TOKEN", "tok-123")]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let cfg: serde_json::Value =
        serde_json::from_str(&read(&s.home.join(".cursor-like/mcp.json"))).unwrap();
    assert_eq!(
        cfg["mcpServers"]["docs"]["headers"]["Authorization"],
        "Bearer tok-123"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(s.home.join(".cursor-like/mcp.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let owned: serde_json::Value = serde_json::from_str(&read(&s.data.join("owned.json"))).unwrap();
    assert_eq!(
        owned["mcp"]["cursor-like"]["secrets_on_disk"],
        serde_json::json!(["docs"])
    );
}

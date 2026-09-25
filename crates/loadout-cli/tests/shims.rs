//! Harness shims and `lo new-source`. Claude Code and Pi
//! can't run in CI, so the tests execute exactly what the shim files tell
//! them to run.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Sandbox, skill};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).unwrap()
}

/// A shell running `command` like an agent would: the sandbox environment
/// with `lo` on PATH.
fn shell(s: &Sandbox, command: &str) -> std::process::Output {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    cmd.env_clear();
    for (k, v) in s.cmd().get_envs() {
        if let Some(v) = v {
            cmd.env(k, v);
        }
    }
    let bin_dir = assert_cmd::cargo::cargo_bin("lo")
        .parent()
        .unwrap()
        .to_path_buf();
    let path = std::env::join_paths(std::iter::once(bin_dir).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    cmd.env("PATH", path);
    cmd.output().unwrap()
}

fn subscribed_sandbox() -> Sandbox {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write("skills/write-spec/SKILL.md", &skill("write-spec", "Specs."));
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    s
}

#[test]
fn claude_code_plugin_manifests_are_valid() {
    let plugin: serde_json::Value =
        serde_json::from_str(&read("shims/claude-code/.claude-plugin/plugin.json")).unwrap();
    assert_eq!(plugin["name"], "loadout");
    let market: serde_json::Value =
        serde_json::from_str(&read(".claude-plugin/marketplace.json")).unwrap();
    assert!(market["owner"]["name"].is_string());
    let entry = &market["plugins"][0];
    assert_eq!(entry["name"], "loadout");
    // Released together with the binary.
    assert_eq!(plugin["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(entry["version"], env!("CARGO_PKG_VERSION"));
    let src = entry["source"].as_str().unwrap();
    assert!(
        repo_root()
            .join(src)
            .join(".claude-plugin/plugin.json")
            .is_file()
    );

    for name in ["loadout", "loadout-authoring"] {
        let skill = read(&format!("shims/claude-code/skills/{name}/SKILL.md"));
        let doc = loadout_model::frontmatter::Document::split(&skill).unwrap();
        let meta: loadout_model::ItemMeta = doc.parse().unwrap();
        assert_eq!(meta.name.as_deref(), Some(name));
        assert!(meta.description.unwrap().len() > 40);
    }
}

/// The `/lo` command's `!` line, with `$ARGUMENTS` filled in.
fn slash_command(arguments: &str) -> String {
    let text = read("shims/claude-code/commands/lo.md");
    let doc = loadout_model::frontmatter::Document::split(&text).unwrap();
    let fm: serde_json::Value = doc.parse().unwrap();
    assert_eq!(fm["allowed-tools"], "Bash(lo *)");
    let line = doc
        .body
        .lines()
        .find(|l| l.starts_with("!`"))
        .expect("an injected command");
    let cmd = line
        .trim_start_matches("!`")
        .trim_end_matches('`')
        .replace("$ARGUMENTS", arguments);
    assert!(cmd.starts_with("lo "), "{cmd}");
    cmd
}

#[test]
fn slash_loadout_why_runs_and_returns_json() {
    let s = subscribed_sandbox();
    s.ok(&["sync"]);
    let out = shell(&s, &slash_command("why skill/write-spec"));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let why: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(why["key"], "skill/write-spec", "{why}");
    assert_eq!(why["winner"], "team-skills:skill/write-spec");

    // Errors come back as JSON with exit 0, so the command never aborts.
    let out = shell(&s, &slash_command("why skill/missing"));
    assert!(out.status.success());
    let err: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(err["error"].is_string(), "{err}");

    // Pending changes (exit 2 normally) don't abort it either.
    let out = shell(&s, &slash_command("status"));
    assert!(out.status.success());
}

#[test]
fn session_start_hook_syncs_when_stale() {
    let s = subscribed_sandbox();
    let hooks: serde_json::Value =
        serde_json::from_str(&read("shims/claude-code/hooks/hooks.json")).unwrap();
    let command = hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(command.contains("--if-stale") && command.contains("--exit-zero"));
    let out = shell(&s, command);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(s.skills_dir().join("write-spec/SKILL.md").is_file());
    // Quiet: nothing on stdout (SessionStart stdout would enter the context).
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
}

#[test]
fn pi_extension_registers_command_and_session_hook() {
    let ts = read("shims/pi/loadout/index.ts");
    assert!(ts.contains("export default function (pi: ExtensionAPI)"));
    assert!(ts.contains("pi.registerCommand(\"lo\""));
    assert!(ts.contains("pi.on(\"session_start\""));
    assert!(ts.contains("\"--exit-zero\""));
}

#[test]
fn github_action_defaults_to_locked_sync() {
    let action = read("shims/github-action/action.yml");
    assert!(action.contains("default: sync --locked --non-interactive"));
    assert!(action.contains("LOADOUT_ROLE"));
}

#[test]
fn new_source_scaffolds_a_repo_that_syncs_and_audits_clean() {
    let s = Sandbox::with_claude();
    let dir = s.root.join("remotes/Payments Skills");
    let r = s.json(&[
        "new-source",
        dir.to_str().unwrap(),
        "--layer",
        "team",
        "--group",
        "payments-dev",
    ]);
    assert_eq!(r["name"], "payments-skills");
    assert_eq!(
        r["created"],
        serde_json::json!([
            "LOADOUT.md",
            "AGENTS.md",
            "skills/example/SKILL.md",
            "mcp/README.md",
            ".github/workflows/loadout-audit.yml"
        ])
    );
    let audit = s.run(&["audit", "--path", dir.to_str().unwrap(), "--json"]);
    assert_eq!(audit.code, 0, "{}", audit.stdout);
    let a: serde_json::Value = serde_json::from_str(&audit.stdout).unwrap();
    assert_eq!(a["findings"], serde_json::json!([]));
    assert_eq!(a["items"], 1);

    // It is a valid source.
    s.git
        .run(Some(&dir), ["init", "--quiet", "--initial-branch", "main"])
        .unwrap();
    s.git.run(Some(&dir), ["add", "--all"]).unwrap();
    s.git
        .run(
            Some(&dir),
            [
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "scaffold",
            ],
        )
        .unwrap();
    s.ok(&["subscribe", dir.to_str().unwrap()]);
    s.ok(&["sync"]);
    let list = s.json(&["list"]);
    assert_eq!(list["items"][0]["id"], "payments-skills:skill/example");
    assert_eq!(list["items"][0]["enabled"], false);

    let again = s.run(&["new-source", dir.to_str().unwrap()]);
    assert_eq!(again.code, 1);
    assert!(again.stderr.contains("already has a LOADOUT.md"));
}

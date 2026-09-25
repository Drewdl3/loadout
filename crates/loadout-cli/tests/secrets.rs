//! A Jira MCP server with a `secret://` reference works in
//! (fake) Claude Code with no secret on disk. A mock stdio MCP server
//! (`examples/mock_mcp.rs`) stands in for Jira; this test plays Claude
//! Code by launching exactly what `~/.claude.json` says.
//!
//! Also a secret-leak grep: the value must not appear in
//! any file under the sandbox (store, lock, config, target configs, logs)
//! nor in any `--json` output.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::Sandbox;

const TOKEN: &str = "jira-s3cr3t-7c1f9e24";
const DOCS_TOKEN: &str = "docs-s3cr3t-b04d2a61";

fn mock_mcp() -> PathBuf {
    let loadout = assert_cmd::cargo::cargo_bin("lo");
    let dir = loadout.parent().unwrap().join("examples");
    let exe = dir.join(format!("mock_mcp{}", std::env::consts::EXE_SUFFIX));
    if !exe.is_file() {
        // `cargo test` builds examples, but `cargo test --test secrets` doesn't.
        let status = Command::new(env!("CARGO"))
            .args(["build", "-q", "-p", "loadout-cli", "--example", "mock_mcp"])
            .status()
            .unwrap();
        assert!(status.success(), "building examples/mock_mcp.rs failed");
    }
    assert!(exe.is_file(), "{} not built", exe.display());
    exe
}

fn jira_item() -> String {
    // Forward slashes keep the YAML string simple on Windows too.
    let cmd = mock_mcp().display().to_string().replace('\\', "/");
    format!(
        r#"---
name: jira
description: Acme Jira.
loadout: {{ mode: required }}
command: "{cmd}"
env:
  JIRA_BASE_URL: "https://acme.example.com"
  JIRA_TOKEN: "secret://jira/token"
---
Connects the agent to Jira.
"#
    )
}

const DOCS: &str = r#"---
name: docs
description: Acme docs search.
url: https://mcp.example.com/docs
headers:
  Authorization: "Bearer secret://docs/token"
  X-Team: payments
---
"#;

fn setup() -> Sandbox {
    let s = Sandbox::with_claude();
    let repo = s.source("acme-tools");
    repo.write("mcp/jira.md", &jira_item())
        .write("mcp/docs.md", DOCS);
    repo.commit("jira and docs");
    s.ok(&["subscribe", &repo.url()]);
    s
}

/// Starts a server the way Claude Code would: from `~/.claude.json`, with
/// the user's (sandboxed) environment.
fn launch(s: &Sandbox, name: &str) -> std::process::Child {
    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(s.home.join(".claude.json")).unwrap())
            .unwrap();
    let entry = &cfg["mcpServers"][name];
    assert_eq!(entry["type"], "stdio");
    assert!(
        entry.get("env").is_none(),
        "no env (and no secret) in config"
    );
    let mut cmd = Command::new(entry["command"].as_str().unwrap());
    for a in entry["args"].as_array().unwrap() {
        cmd.arg(a.as_str().unwrap());
    }
    let base = s.cmd();
    let std_cmd = base.get_envs();
    cmd.env_clear();
    for (k, v) in std_cmd {
        if let Some(v) = v {
            cmd.env(k, v);
        }
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn rpc(
    stdin: &mut impl Write,
    stdout: &mut impl BufRead,
    id: u64,
    method: &str,
) -> serde_json::Value {
    let req = serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": {}});
    writeln!(stdin, "{req}").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap_or_else(|e| panic!("bad response {line:?}: {e}"))
}

#[test]
fn jira_mcp_works_with_no_secret_on_disk() {
    let s = setup();
    let set = s.run_stdin(&["secrets", "set", "jira/token"], &format!("{TOKEN}\n"));
    assert_eq!(set.code, 0, "{}", set.stderr);
    let out = s.run_env(&["sync"], &[("DOCS_TOKEN", DOCS_TOKEN)]);
    assert_eq!(out.code, 0, "{}", out.stderr);

    let check = s.run_env(
        &["secrets", "check", "--json"],
        &[("DOCS_TOKEN", DOCS_TOKEN)],
    );
    assert_eq!(check.code, 0, "{}\n{}", check.stdout, check.stderr);
    let report: serde_json::Value = serde_json::from_str(&check.stdout).unwrap();
    let statuses: Vec<(&str, &str)> = report["references"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["reference"].as_str().unwrap(),
                r["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        statuses,
        [("secret://docs/token", "ok"), ("secret://jira/token", "ok")]
    );

    // Claude Code launches the server; it answers with the token it got.
    let mut child = launch(&s, "jira");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let init = rpc(&mut stdin, &mut stdout, 1, "initialize");
    assert_eq!(init["result"]["serverInfo"]["name"], "acme-jira-mock");
    let tools = rpc(&mut stdin, &mut stdout, 2, "tools/list");
    assert_eq!(tools["result"]["tools"][0]["name"], "whoami");
    let call = rpc(&mut stdin, &mut stdout, 3, "tools/call");
    assert_eq!(
        call["result"]["content"][0]["text"],
        format!("base=https://acme.example.com token={TOKEN}")
    );
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success());

    // HTTP: Claude Code's headersHelper prints the secret header.
    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(s.home.join(".claude.json")).unwrap())
            .unwrap();
    let helper = cfg["mcpServers"]["docs"]["headersHelper"].as_str().unwrap();
    let mut sh = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(helper);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(helper);
        c
    };
    sh.env_clear();
    for (k, v) in s.cmd().get_envs() {
        if let Some(v) = v {
            sh.env(k, v);
        }
    }
    sh.env("DOCS_TOKEN", DOCS_TOKEN);
    let out = sh.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let headers: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        headers,
        serde_json::json!({ "Authorization": format!("Bearer {DOCS_TOKEN}") })
    );

    // No secret value in any file or any --json output.
    let mut outputs = Vec::new();
    for args in [
        &["sync", "--json"][..],
        &["list", "--json"],
        &["why", "mcp/jira", "--json"],
        &["doctor", "--json"],
        &["secrets", "check", "--json"],
        &["export", "--json"],
        &["status", "--json"],
        &["audit", "--json"],
    ] {
        let o = s.run_env(args, &[("DOCS_TOKEN", DOCS_TOKEN)]);
        outputs.push(format!("{args:?}\n{}\n{}", o.stdout, o.stderr));
    }
    for o in &outputs {
        assert!(!o.contains(TOKEN) && !o.contains(DOCS_TOKEN), "leak in {o}");
    }
    let leaks = files_containing(&s.root, &[TOKEN, DOCS_TOKEN], &s.keystore_file());
    assert!(leaks.is_empty(), "secret found in {leaks:#?}");
}

/// Every file under `dir` (except `skip`, the stand-in keychain) whose bytes
/// contain one of `needles`.
fn files_containing(dir: &Path, needles: &[&str], skip: &Path) -> Vec<PathBuf> {
    let mut hits = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            let ft = e.file_type().unwrap();
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() && p != skip {
                let bytes = std::fs::read(&p).unwrap();
                if needles
                    .iter()
                    .any(|n| bytes.windows(n.len()).any(|w| w == n.as_bytes()))
                {
                    hits.push(p);
                }
            }
        }
    }
    hits
}

#[test]
fn check_reports_missing_and_set_clear_round_trip() {
    let s = setup();
    s.ok(&["sync"]);
    let out = s.run(&["secrets", "check", "--json"]);
    assert_eq!(out.code, 1);
    let r: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(r["references"][1]["status"], "missing");
    assert_eq!(
        r["references"][1]["items"],
        serde_json::json!(["acme-tools:mcp/jira"])
    );
    let detail = r["references"][1]["detail"].as_str().unwrap();
    assert!(detail.contains("env://JIRA_TOKEN"), "{detail}");

    let human = s.run(&["secrets"]);
    s.insta()
        .bind(|| insta::assert_snapshot!("check_missing", human.stdout));

    s.run_stdin(&["secrets", "set", "jira/token"], "abc\n");
    let out = s.run_env(&["secrets", "check"], &[("DOCS_TOKEN", "x")]);
    assert_eq!(out.code, 0, "{}", out.stdout);

    let cleared = s.json(&["secrets", "clear", "jira/token"]);
    assert_eq!(cleared["changed"], true);
    let again = s.json(&["secrets", "clear", "jira/token"]);
    assert_eq!(again["changed"], false);
    assert!(s.run(&["secrets", "set", "bad name"]).code != 0);
}

#[test]
fn mcp_run_fails_cleanly_without_secret() {
    let s = setup();
    s.ok(&["sync"]);
    let out = s.run(&["mcp-run", "acme-tools:mcp/jira"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stdout, "", "stdout belongs to the MCP protocol");
    assert!(
        out.stderr.contains("secret://jira/token: not found"),
        "{}",
        out.stderr
    );
    let out = s.run(&["mcp-run", "acme-tools:mcp/nope"]);
    assert!(out.stderr.contains("is not installed"), "{}", out.stderr);
}

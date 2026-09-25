//! A minimal stdio MCP server for end-to-end tests (built by `cargo test`,
//! never shipped). It stands in for a Jira MCP server: one tool, `whoami`,
//! answers with the `JIRA_BASE_URL` and `JIRA_TOKEN` it was started with,
//! proving the launch wrapper delivered the secret in memory.
//!
//! Speaks newline-delimited JSON-RPC 2.0 (the MCP stdio transport).

use std::io::{BufRead, Write};

use serde_json::{Value, json};

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = msg.get("id").cloned() else {
            continue; // notification
        };
        let result = match msg["method"].as_str().unwrap_or_default() {
            "initialize" => json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "acme-jira-mock", "version": "0.0.0" }
            }),
            "tools/list" => json!({
                "tools": [{
                    "name": "whoami",
                    "description": "Report the Jira credentials this server received.",
                    "inputSchema": { "type": "object", "properties": {} }
                }]
            }),
            "tools/call" => {
                let base = std::env::var("JIRA_BASE_URL").unwrap_or_default();
                let token = std::env::var("JIRA_TOKEN").unwrap_or_default();
                json!({
                    "content": [{ "type": "text", "text": format!("base={base} token={token}") }]
                })
            }
            other => {
                let err = json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": { "code": -32601, "message": format!("unknown method {other}") }
                });
                let _ = writeln!(stdout, "{err}");
                let _ = stdout.flush();
                continue;
            }
        };
        let _ = writeln!(
            stdout,
            "{}",
            json!({ "jsonrpc": "2.0", "id": id, "result": result })
        );
        let _ = stdout.flush();
    }
}

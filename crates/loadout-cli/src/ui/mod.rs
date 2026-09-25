//! `lo ui`: a local web dashboard over the same engine as
//! the CLI.
//!
//! - Binds `127.0.0.1` only and checks the `Host` header (no DNS
//!   rebinding); every API call needs the random per-run token.
//! - Read endpoints and actions run this `lo` binary with `--json`, so
//!   the UI can do exactly what the CLI can, with the same config writes.

pub mod export;

use std::fmt::Write as _;
use std::io::Read;
use std::process::Command;

use anyhow::{Context, Result};
use clap::Args;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::ctx::{Ctx, Report};
use crate::exit;

const INDEX: &str = include_str!("index.html");
/// Largest request body accepted.
const MAX_BODY: u64 = 1 << 20;

#[derive(Debug, Args)]
pub struct UiArgs {
    /// Port on 127.0.0.1 (default: a free one).
    #[arg(long, default_value_t = 0)]
    pub port: u16,
    /// Don't open a browser; just print the URL.
    #[arg(long)]
    pub no_open: bool,
}

/// Printed once the server listens (`--json`: one line).
#[derive(Debug, Serialize, JsonSchema)]
pub struct UiStarted {
    /// Open this URL (it carries the per-run token).
    pub url: String,
    pub port: u16,
}

impl Report for UiStarted {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        writeln!(out, "Loadout UI: {}", self.url)?;
        writeln!(out, "Press Ctrl-C to stop.")
    }
}

/// Runs the server until the process is stopped.
pub fn run(ctx: &Ctx, args: UiArgs) -> Result<u8> {
    let server = Server::http(("127.0.0.1", args.port))
        .map_err(|e| anyhow::anyhow!("listening on 127.0.0.1:{}: {e}", args.port))?;
    let port = server
        .server_addr()
        .to_ip()
        .context("server has no IP address")?
        .port();
    let token = new_token()?;
    let url = format!("http://127.0.0.1:{port}/?token={token}");
    if ctx.json {
        println!(
            "{}",
            serde_json::to_string(&UiStarted {
                url: url.clone(),
                port
            })?
        );
    } else {
        ctx.emit(&UiStarted {
            url: url.clone(),
            port,
        })?;
    }
    use std::io::Write as _;
    std::io::stdout().flush()?;
    if !args.no_open {
        open_browser(&url);
    }
    let ui = Ui { ctx, token, port };
    for req in server.incoming_requests() {
        ui.handle(req);
    }
    Ok(exit::OK)
}

struct Ui<'a> {
    ctx: &'a Ctx,
    token: String,
    port: u16,
}

type Reply = Response<std::io::Cursor<Vec<u8>>>;

fn json_reply(status: u16, v: &Value) -> Reply {
    Response::from_data(serde_json::to_vec(v).unwrap_or_default())
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json"))
        .with_header(header("Cache-Control", "no-store"))
}

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("valid header")
}

impl Ui<'_> {
    fn handle(&self, mut req: Request) {
        let reply = self.route(&mut req);
        let _ = req.respond(reply);
    }

    fn host_ok(&self, req: &Request) -> bool {
        let host = req
            .headers()
            .iter()
            .find(|h| h.field.equiv("Host"))
            .map(|h| h.value.as_str().to_owned());
        let allowed = [
            format!("127.0.0.1:{}", self.port),
            format!("localhost:{}", self.port),
        ];
        host.is_some_and(|h| allowed.contains(&h))
    }

    fn token_ok(&self, req: &Request) -> bool {
        req.headers()
            .iter()
            .find(|h| h.field.equiv("X-Loadout-Token"))
            .is_some_and(|h| constant_eq(h.value.as_str(), &self.token))
    }

    fn route(&self, req: &mut Request) -> Reply {
        if !self.host_ok(req) {
            return json_reply(403, &json!({"error": "bad Host header"}));
        }
        let url = req.url().to_owned();
        let (path, query) = url.split_once('?').unwrap_or((&url, ""));
        let params = parse_query(query);
        if path == "/" && *req.method() == Method::Get {
            if params
                .iter()
                .any(|(k, v)| k == "token" && constant_eq(v, &self.token))
            {
                return Response::from_data(INDEX.as_bytes().to_vec())
                    .with_header(header("Content-Type", "text/html; charset=utf-8"))
                    .with_header(header("Cache-Control", "no-store"))
                    .with_header(header(
                        "Content-Security-Policy",
                        "default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'",
                    ));
            }
            return json_reply(403, &json!({"error": "open the URL printed by `lo ui`"}));
        }
        if !path.starts_with("/api/") {
            return json_reply(404, &json!({"error": "not found"}));
        }
        if !self.token_ok(req) {
            return json_reply(401, &json!({"error": "missing or wrong X-Loadout-Token"}));
        }
        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        match (req.method().clone(), path) {
            (Method::Get, "/api/status") => self.loadout(&["status"]),
            (Method::Get, "/api/profile") => self.loadout(&["profile"]),
            (Method::Get, "/api/list") => self.loadout(&["list"]),
            (Method::Get, "/api/doctor") => self.loadout(&["doctor"]),
            (Method::Get, "/api/diff") => self.loadout(&["diff"]),
            (Method::Get, "/api/targets") => self.loadout(&["targets"]),
            (Method::Get, "/api/sources") => self.sources(),
            (Method::Get, "/api/layers") => match layers_json(self.ctx) {
                Ok(v) => json_reply(200, &json!({"code": 0, "output": v})),
                Err(e) => json_reply(500, &json!({"error": format!("{e:#}")})),
            },
            (Method::Get, "/api/why") => match get("key") {
                Some(k) if !k.starts_with('-') => self.loadout(&["why", &k]),
                _ => json_reply(400, &json!({"error": "?key=kind/name required"})),
            },
            (Method::Get, "/api/info") => match get("what") {
                Some(w) if !w.starts_with('-') => self.loadout(&["info", &w]),
                _ => json_reply(400, &json!({"error": "?what= required"})),
            },
            (Method::Get, "/api/search") => {
                let q = get("q").unwrap_or_default();
                if q.starts_with('-') {
                    return json_reply(400, &json!({"error": "bad query"}));
                }
                // Everything that matches; the page filters by facets.
                let mut args = vec!["search", &q, "--limit", "0"];
                if get("all").is_some() {
                    args.push("--all-sources");
                }
                self.loadout(&args)
            }
            (Method::Get, "/api/preview") => match get("url") {
                Some(u) if !u.trim().is_empty() && !u.starts_with('-') => {
                    self.loadout(&["subscribe", u.trim(), "--dry-run"])
                }
                _ => json_reply(400, &json!({"error": "?url= required"})),
            },
            (Method::Get, "/api/templates") => {
                if get("all").is_some() {
                    self.loadout(&["template", "list", "--all-sources"])
                } else {
                    self.loadout(&["template", "list"])
                }
            }
            (Method::Get, "/api/adopt") => match get("path") {
                Some(p) if p.starts_with('-') => json_reply(400, &json!({"error": "bad path"})),
                Some(p) if !p.trim().is_empty() => self.loadout(&["adopt", p.trim()]),
                _ => self.loadout(&["adopt"]),
            },
            (Method::Post, "/api/adopt") => match read_json(req) {
                Ok(body) => self.adopt(&body),
                Err(e) => json_reply(400, &json!({"error": e})),
            },
            (Method::Post, "/api/template-use") => match read_json(req) {
                Ok(body) => self.template_use(&body),
                Err(e) => json_reply(400, &json!({"error": e})),
            },
            (Method::Post, "/api/run") => match read_json(req) {
                Ok(body) => self.action(&body),
                Err(e) => json_reply(400, &json!({"error": e})),
            },
            (Method::Get, "/api/source-item") => export::read_item(self.ctx, &params),
            (Method::Post, "/api/source-item") => match read_json(req) {
                Ok(body) => export::write_item(self.ctx, &body),
                Err(e) => json_reply(400, &json!({"error": e})),
            },
            (Method::Get, "/api/source-template") => export::read_template(self.ctx, &params),
            (Method::Post, "/api/source-template") => match read_json(req) {
                Ok(body) => export::write_template(self.ctx, &body),
                Err(e) => json_reply(400, &json!({"error": e})),
            },
            (Method::Get, "/api/whoami") => {
                let name = export::git_user(&loadout_git::Git::new(), None);
                json_reply(200, &json!({"code": 0, "output": {"name": name}}))
            }
            (Method::Post, "/api/copy-item") => match read_json(req) {
                Ok(b) => export::copy_item(self.ctx, &b),
                Err(e) => json_reply(400, &json!({"error": e})),
            },
            (Method::Post, "/api/move-item") => match read_json(req) {
                Ok(body) => export::move_item(self.ctx, &body),
                Err(e) => json_reply(400, &json!({"error": e})),
            },
            (Method::Post, "/api/export") => match read_json(req) {
                Ok(body) => self.export(&body),
                Err(e) => json_reply(400, &json!({"error": e})),
            },
            _ => json_reply(404, &json!({"error": "not found"})),
        }
    }

    /// Runs `lo <args> --json` (the same binary, same environment).
    fn loadout(&self, args: &[&str]) -> Reply {
        let (code, output) = run_loadout(self.ctx, args);
        json_reply(200, &json!({ "code": code, "output": output }))
    }

    /// `POST /api/run {"args": [...]}`: the configuring commands only.
    fn action(&self, body: &Value) -> Reply {
        let Some(args) = body["args"].as_array() else {
            return json_reply(400, &json!({"error": "{\"args\": [...]} expected"}));
        };
        let args: Vec<&str> = args.iter().filter_map(Value::as_str).collect();
        if let Err(e) = allowed(&args) {
            return json_reply(403, &json!({ "error": e }));
        }
        self.loadout(&args)
    }

    fn export(&self, body: &Value) -> Reply {
        let Some(source) = body["source"].as_str().filter(|s| !s.starts_with('-')) else {
            return json_reply(400, &json!({"error": "\"source\" required"}));
        };
        let mut args = vec!["export-source", source];
        let msg = body["message"].as_str().unwrap_or("Update from lo ui");
        args.extend(["--message", msg]);
        self.loadout(&args)
    }

    /// `POST /api/template-use {"template", "into", "name"?, "values": {}}`.
    fn template_use(&self, body: &Value) -> Reply {
        let arg = |k: &str| {
            body[k]
                .as_str()
                .filter(|v| !v.is_empty() && !v.starts_with('-'))
        };
        let (Some(template), Some(into)) = (arg("template"), arg("into")) else {
            return json_reply(400, &json!({"error": "\"template\" and \"into\" required"}));
        };
        let mut args: Vec<String> = ["template", "use", template, "--into", into]
            .map(str::to_owned)
            .to_vec();
        if let Some(name) = arg("name") {
            args.extend(["--name".to_owned(), name.to_owned()]);
        }
        if let Some(values) = body["values"].as_object() {
            for (k, v) in values {
                let valid = !k.is_empty()
                    && k.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    && !k.starts_with('-');
                let Some(v) = v.as_str().filter(|_| valid) else {
                    return json_reply(400, &json!({"error": format!("bad value for {k:?}")}));
                };
                args.extend(["--set".to_owned(), format!("{k}={v}")]);
            }
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.loadout(&args)
    }

    /// `POST /api/adopt {"into", "skills": [...], "path"?}`.
    fn adopt(&self, body: &Value) -> Reply {
        let ok = |v: &&str| !v.is_empty() && !v.starts_with('-');
        let Some(into) = body["into"].as_str().filter(ok) else {
            return json_reply(400, &json!({"error": "\"into\" required"}));
        };
        let skills: Vec<&str> = body["skills"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if skills.is_empty() || !skills.iter().all(ok) {
            return json_reply(400, &json!({"error": "\"skills\": [names] required"}));
        }
        let mut args = vec!["adopt"];
        match body["path"].as_str().map(str::trim) {
            Some(p) if p.starts_with('-') => {
                return json_reply(400, &json!({"error": "bad path"}));
            }
            Some(p) if !p.is_empty() => args.push(p),
            _ => {}
        }
        args.extend(["--into", into]);
        for s in &skills {
            args.extend(["--skill", s]);
        }
        self.loadout(&args)
    }

    fn sources(&self) -> Reply {
        match sources_json(self.ctx) {
            Ok(v) => json_reply(200, &json!({"code": 0, "output": v})),
            Err(e) => json_reply(500, &json!({"error": format!("{e:#}")})),
        }
    }
}

/// Synced sources (with `manual`, `via`, `upstream`) and the company config URL.
pub fn sources_json(ctx: &Ctx) -> Result<Value> {
    let resolved = crate::state::Resolved::load(&ctx.paths.resolved_file())?;
    let config = ctx.load_config()?;
    let manual: Vec<String> = config
        .sources
        .iter()
        .map(|s| loadout_model::config::normalize_url(&s.url))
        .collect();
    let sources: Vec<Value> = resolved
        .map(|r| r.sources)
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let is_manual = manual.contains(&loadout_model::config::normalize_url(&s.url));
            json!({
                "name": s.name,
                "url": s.url,
                "commit": s.commit,
                "manual": is_manual,
                "via": s.via,
                "upstream": s.upstream,
                "layer": s.layer,
                "group": s.group,
            })
        })
        .collect();
    Ok(json!({"sources": sources, "company_config": config.company_config}))
}

/// The layers in rank order, broadest first, and the winner of
/// each `kind/name` after the last sync, for the Layers page.
pub fn layers_json(ctx: &Ctx) -> Result<Value> {
    let config = ctx.load_config()?;
    let layers = match &config.company_config {
        Some(url) if ctx.paths.project.is_none() => {
            crate::membership::load_company_config(ctx, &loadout_git::Git::new(), url, false)
                .map(|c| c.company_config.layer_model())
                .unwrap_or_default()
        }
        _ => loadout_model::LayerModel::default(),
    };
    let mut ranked: Vec<(&str, i64)> = layers.layers().collect();
    ranked.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(b.0)));
    let winners: serde_json::Map<String, Value> =
        crate::state::Resolved::load(&ctx.paths.resolved_file())?
            .map(|r| r.resolution.items)
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                let rule = r
                    .rule
                    .map(|x| serde_json::to_value(x).unwrap_or(Value::Null));
                (
                    r.key.to_string(),
                    json!({
                        "winner": r.winner.map(|w| w.to_string()),
                        "rule": rule,
                        "enabled": r.enabled,
                    }),
                )
            })
            .collect();
    Ok(json!({
        "layers": ranked.iter().map(|(n, r)| json!({"name": n, "rank": r})).collect::<Vec<_>>(),
        "resolution": winners,
    }))
}

/// The configuring commands the UI may run, with their argument shapes.
pub fn allowed(args: &[&str]) -> Result<(), String> {
    let positional = |n: usize| {
        args.len() == n + 1
            && args[1..]
                .iter()
                .all(|a| !a.is_empty() && !a.starts_with('-'))
    };
    let ok = match args.first().copied() {
        Some("enable" | "disable" | "prefer" | "join" | "leave" | "subscribe" | "unsubscribe") => {
            positional(1)
        }
        Some("sync") => args.len() == 1,
        // Joining a company config from the UI's Start page.
        Some("init") => {
            args.len() == 3
                && !args[1].is_empty()
                && !args[1].starts_with('-')
                && args[2] == "--non-interactive"
        }
        Some("approve") => positional(1) || args == ["approve", "--all"],
        Some("profile") => args == ["profile", "refresh"],
        Some("targets") => match args.get(1).copied() {
            Some("enable" | "disable") => positional(2),
            Some("mode") => {
                args.len() == 4
                    && !args[2].starts_with('-')
                    && matches!(args[3], "symlink" | "copy")
            }
            _ => false,
        },
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("the UI can't run `lo {}`", args.join(" ")))
    }
}

/// Runs this binary with `args` + `--json`; returns (exit code, JSON).
pub fn run_loadout(ctx: &Ctx, args: &[&str]) -> (i32, Value) {
    let exe = std::env::current_exe().unwrap_or_else(|_| "lo".into());
    let mut cmd = Command::new(exe);
    if let Some(p) = &ctx.paths.project {
        cmd.arg("--project-dir").arg(&p.root);
    }
    cmd.args(args)
        .arg("--json")
        .stdin(std::process::Stdio::null());
    match cmd.output() {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let v = serde_json::from_str(&text).unwrap_or_else(
                |_| json!({ "error": String::from_utf8_lossy(&out.stderr).trim().to_owned() }),
            );
            (out.status.code().unwrap_or(1), v)
        }
        Err(e) => (1, json!({ "error": e.to_string() })),
    }
}

fn read_json(req: &mut Request) -> Result<Value, String> {
    let mut body = String::new();
    req.as_reader()
        .take(MAX_BODY)
        .read_to_string(&mut body)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| format!("invalid JSON: {e}"))
}

pub fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(v) => {
                    out.push(v);
                    i += 2;
                }
                Err(_) => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn new_token() -> Result<String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn open_browser(url: &str) {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]).arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    let _ = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist() {
        for ok in [
            &["enable", "a:skill/x"][..],
            &["prefer", "a:skill/x"],
            &["join", "team:payments-dev"],
            &["subscribe", "https://git.example.com/x"],
            &["sync"],
            &["approve", "--all"],
            &["approve", "payments-skills"],
            &["profile", "refresh"],
            &["targets", "enable", "pi"],
            &["targets", "mode", "claude-code", "copy"],
            &[
                "init",
                "https://git.example.com/acme/company-config",
                "--non-interactive",
            ],
        ] {
            assert!(allowed(ok).is_ok(), "{ok:?}");
        }
        for bad in [
            &["enable"][..],
            &["enable", "--project-dir"],
            &["enable", "a", "b"],
            &["sync", "--force-audit"],
            &["secrets", "set", "x"],
            &["mcp-run", "a:mcp/x"],
            &["import", "lo1_x"],
            &["targets", "mode", "claude-code", "hardlink"],
            &["init", "https://git.example.com/acme/company-config"],
            &["init", "--new-source", "--non-interactive"],
            &["init", "x", "--new-company"],
            &[],
        ] {
            assert!(allowed(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn query_decoding_and_tokens() {
        assert_eq!(
            parse_query("key=skill%2Fwrite-spec&q=a+b&x"),
            [
                ("key".to_owned(), "skill/write-spec".to_owned()),
                ("q".to_owned(), "a b".to_owned()),
                ("x".to_owned(), String::new())
            ]
        );
        assert_eq!(decode("100%"), "100%");
        assert!(constant_eq("abc", "abc") && !constant_eq("abc", "abd") && !constant_eq("a", "ab"));
        let t = new_token().unwrap();
        assert_eq!(t.len(), 48);
        assert_ne!(t, new_token().unwrap());
    }
}

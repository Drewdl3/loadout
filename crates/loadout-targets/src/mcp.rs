//! MCP renderers: merge Loadout-managed servers into a
//! target's native config file without touching anything else.
//!
//! A renderer is named in the target table (`[mcp] renderer = "…"`). Each
//! one knows its file format, its default key, what it can express
//! ([`Caps`]), and how to spell a server. Rendering is split in two:
//! [`Format::render`] turns a [`Delivered`] server into the native shape,
//! and [`merge`] edits the file text. Both are pure; [`apply`] does the
//! atomic, backed-up write.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use loadout_core::mcp::{Caps, Delivered, EnvSupport, Piece, Value};
use loadout_model::Transport;
use serde_json::{Map, Value as Json, json};

use crate::fsutil::{WriteOptions, atomic_write_opts};

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("unknown MCP renderer {0:?}")]
    UnknownRenderer(String),
    #[error("{path} is not valid {format} ({message}); Loadout left it unchanged")]
    Parse {
        path: String,
        format: &'static str,
        message: String,
    },
    #[error("{path}: `{key}` is not a table/object; Loadout left it unchanged")]
    NotAnObject { path: String, key: String },
    #[error("writing {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// A native MCP config format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `~/.claude.json` `mcpServers`.
    ClaudeCodeJson,
    /// `~/.cursor/mcp.json` `mcpServers`.
    CursorJson,
    /// `pi-mcp-adapter`'s `~/.pi/agent/mcp.json` `mcpServers`.
    PiMcpJson,
    /// `~/.config/opencode/opencode.json` `mcp`.
    OpencodeJson,
    /// `~/.codex/config.toml` `[mcp_servers.<name>]`.
    CodexToml,
    /// IBM Bob's `~/.bob/mcp_settings.json` `mcpServers` (`url` = SSE,
    /// `httpURL` = streamable HTTP; no documented env interpolation).
    BobJson,
}

impl Format {
    pub fn from_renderer(name: &str) -> Result<Self, McpError> {
        Ok(match name {
            "claude-code-json" => Format::ClaudeCodeJson,
            "cursor-json" => Format::CursorJson,
            "pi-mcp-json" => Format::PiMcpJson,
            "opencode-json" => Format::OpencodeJson,
            "codex-toml" => Format::CodexToml,
            "bob-json" => Format::BobJson,
            other => return Err(McpError::UnknownRenderer(other.to_owned())),
        })
    }

    /// The key holding servers when the target table doesn't name one.
    pub fn default_key(self) -> &'static str {
        match self {
            Format::OpencodeJson => "mcp",
            Format::CodexToml => "mcp_servers",
            _ => "mcpServers",
        }
    }

    pub fn caps(self) -> Caps {
        match self {
            Format::ClaudeCodeJson => Caps {
                env: EnvSupport::Interpolate,
                headers_helper: true,
                sse: true,
            },
            Format::CursorJson | Format::PiMcpJson | Format::OpencodeJson => Caps {
                env: EnvSupport::Interpolate,
                headers_helper: false,
                sse: true,
            },
            Format::CodexToml => Caps {
                env: EnvSupport::WholeHeader,
                headers_helper: false,
                sse: false,
            },
            Format::BobJson => Caps {
                env: EnvSupport::None,
                headers_helper: false,
                sse: true,
            },
        }
    }

    /// The native entry for one server. `server` must not contain
    /// [`Piece::Secret`]s (resolve them first).
    pub fn render(self, server: &Delivered) -> Json {
        match server {
            Delivered::Stdio { command, args, env } => self.render_stdio(command, args, env),
            Delivered::Remote {
                transport,
                url,
                headers,
                headers_helper,
            } => self.render_remote(*transport, url, headers, headers_helper.as_deref()),
        }
    }

    fn render_stdio(self, command: &str, args: &[String], env: &BTreeMap<String, String>) -> Json {
        let mut m = Map::new();
        match self {
            Format::OpencodeJson => {
                m.insert("type".into(), json!("local"));
                let mut cmd = vec![command.to_owned()];
                cmd.extend(args.iter().cloned());
                m.insert("command".into(), json!(cmd));
                if !env.is_empty() {
                    m.insert("environment".into(), json!(env));
                }
                m.insert("enabled".into(), json!(true));
            }
            _ => {
                if matches!(self, Format::ClaudeCodeJson | Format::CursorJson) {
                    m.insert("type".into(), json!("stdio"));
                }
                m.insert("command".into(), json!(command));
                if !args.is_empty() {
                    m.insert("args".into(), json!(args));
                }
                if !env.is_empty() {
                    m.insert("env".into(), json!(env));
                }
            }
        }
        Json::Object(m)
    }

    fn render_remote(
        self,
        transport: Transport,
        url: &Value,
        headers: &BTreeMap<String, Value>,
        helper: Option<&str>,
    ) -> Json {
        let mut m = Map::new();
        match self {
            Format::ClaudeCodeJson => {
                m.insert("type".into(), json!(transport.as_str()));
            }
            Format::OpencodeJson => {
                m.insert("type".into(), json!("remote"));
            }
            _ => {}
        }
        let url_key = match (self, transport) {
            (Format::BobJson, Transport::Http) => "httpURL",
            _ => "url",
        };
        m.insert(url_key.into(), json!(self.spell(url)));
        if self == Format::CodexToml {
            let (mut plain, mut from_env) = (Map::new(), Map::new());
            for (name, v) in headers {
                match v.0.as_slice() {
                    [Piece::Env(var)] => {
                        from_env.insert(name.clone(), json!(var));
                    }
                    [Piece::Lit(prefix), Piece::Env(var)]
                        if prefix == "Bearer " && name.eq_ignore_ascii_case("authorization") =>
                    {
                        m.insert("bearer_token_env_var".into(), json!(var));
                    }
                    _ => {
                        plain.insert(name.clone(), json!(self.spell(v)));
                    }
                }
            }
            if !plain.is_empty() {
                m.insert("http_headers".into(), Json::Object(plain));
            }
            if !from_env.is_empty() {
                m.insert("env_http_headers".into(), Json::Object(from_env));
            }
        } else if !headers.is_empty() {
            let h: Map<String, Json> = headers
                .iter()
                .map(|(k, v)| (k.clone(), json!(self.spell(v))))
                .collect();
            m.insert("headers".into(), Json::Object(h));
        }
        if let Some(helper) = helper {
            m.insert("headersHelper".into(), json!(helper));
        }
        if self == Format::OpencodeJson {
            m.insert("enabled".into(), json!(true));
        }
        Json::Object(m)
    }

    /// Spells a value in the format's env-interpolation syntax.
    fn spell(self, v: &Value) -> String {
        let mut out = String::new();
        for p in &v.0 {
            match p {
                Piece::Lit(l) => out.push_str(l),
                Piece::Env(var) => match self {
                    Format::CursorJson => out.push_str(&format!("${{env:{var}}}")),
                    Format::OpencodeJson => out.push_str(&format!("{{env:{var}}}")),
                    _ => out.push_str(&format!("${{{var}}}")),
                },
                // Callers resolve secrets before rendering; never emit a
                // reference as if it were a value.
                Piece::Secret(r) => out.push_str(&r.to_string()),
            }
        }
        out
    }

    fn name(self) -> &'static str {
        match self {
            Format::CodexToml => "TOML",
            _ => "JSON",
        }
    }
}

/// What [`merge`] did to the managed servers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Merged {
    /// The new file text, or `None` when nothing changed.
    pub text: Option<String>,
    /// Servers Loadout now owns in this file.
    pub owned: BTreeSet<String>,
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: usize,
    /// Desired servers skipped because an unowned entry has that name.
    pub collisions: Vec<String>,
}

/// Merges `desired` servers into `text` (the current file contents, empty
/// if missing). Only names in `owned` or `desired` are touched; an unowned
/// entry with a desired name is left alone (collision) unless it already
/// equals what we'd write, in which case it is adopted.
pub fn merge(
    format: Format,
    path_display: &str,
    text: &str,
    key: &str,
    desired: &BTreeMap<String, Json>,
    owned: &BTreeSet<String>,
) -> Result<Merged, McpError> {
    match format {
        Format::CodexToml => merge_toml(path_display, text, key, desired, owned),
        _ => merge_json(format, path_display, text, key, desired, owned),
    }
}

fn plan_changes(
    existing: &BTreeMap<String, Json>,
    desired: &BTreeMap<String, Json>,
    owned: &BTreeSet<String>,
) -> (Merged, Vec<(String, Option<Json>)>) {
    let mut m = Merged::default();
    let mut edits = Vec::new();
    for name in owned {
        if !desired.contains_key(name) && existing.contains_key(name) {
            m.removed.push(name.clone());
            edits.push((name.clone(), None));
        }
    }
    for (name, want) in desired {
        match existing.get(name) {
            None => {
                m.added.push(name.clone());
                edits.push((name.clone(), Some(want.clone())));
                m.owned.insert(name.clone());
            }
            Some(have) if have == want => {
                m.unchanged += 1;
                m.owned.insert(name.clone());
            }
            Some(_) if owned.contains(name) => {
                m.updated.push(name.clone());
                edits.push((name.clone(), Some(want.clone())));
                m.owned.insert(name.clone());
            }
            Some(_) => m.collisions.push(name.clone()),
        }
    }
    (m, edits)
}

fn merge_json(
    format: Format,
    path: &str,
    text: &str,
    key: &str,
    desired: &BTreeMap<String, Json>,
    owned: &BTreeSet<String>,
) -> Result<Merged, McpError> {
    let mut root: Json = if text.trim().is_empty() {
        Json::Object(Map::new())
    } else {
        serde_json::from_str(text).map_err(|e| McpError::Parse {
            path: path.to_owned(),
            format: format.name(),
            message: e.to_string(),
        })?
    };
    let not_obj = || McpError::NotAnObject {
        path: path.to_owned(),
        key: key.to_owned(),
    };
    let obj = root.as_object_mut().ok_or_else(|| McpError::NotAnObject {
        path: path.to_owned(),
        key: "(root)".into(),
    })?;
    let existing: BTreeMap<String, Json> = match obj.get(key) {
        None => BTreeMap::new(),
        Some(Json::Object(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Some(_) => return Err(not_obj()),
    };
    let (mut merged, edits) = plan_changes(&existing, desired, owned);
    if edits.is_empty() {
        return Ok(merged);
    }
    let servers = obj
        .entry(key.to_owned())
        .or_insert_with(|| Json::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(not_obj)?;
    for (name, v) in edits {
        match v {
            Some(v) => {
                servers.insert(name, v);
            }
            None => {
                servers.shift_remove(&name);
            }
        }
    }
    let mut out = serde_json::to_string_pretty(&root).expect("JSON values serialize");
    if text.is_empty() || text.ends_with('\n') {
        out.push('\n');
    }
    merged.text = Some(out);
    Ok(merged)
}

fn merge_toml(
    path: &str,
    text: &str,
    key: &str,
    desired: &BTreeMap<String, Json>,
    owned: &BTreeSet<String>,
) -> Result<Merged, McpError> {
    use toml_edit::{DocumentMut, Item, Table};
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| McpError::Parse {
            path: path.to_owned(),
            format: "TOML",
            message: e.to_string(),
        })?;
    let not_obj = || McpError::NotAnObject {
        path: path.to_owned(),
        key: key.to_owned(),
    };
    let existing: BTreeMap<String, Json> = match doc.get(key) {
        None => BTreeMap::new(),
        Some(item) => item
            .as_table_like()
            .ok_or_else(not_obj)?
            .iter()
            .map(|(k, v)| (k.to_owned(), toml_to_json(v)))
            .collect(),
    };
    let (mut merged, edits) = plan_changes(&existing, desired, owned);
    if edits.is_empty() {
        return Ok(merged);
    }
    if doc.get(key).is_none() {
        let mut t = Table::new();
        t.set_implicit(true);
        doc.insert(key, Item::Table(t));
    }
    let servers = doc[key].as_table_like_mut().ok_or_else(not_obj)?;
    for (name, v) in edits {
        match v {
            Some(v) => {
                servers.insert(&name, Item::Table(json_to_toml_table(&v)));
            }
            None => {
                servers.remove(&name);
            }
        }
    }
    merged.text = Some(doc.to_string());
    Ok(merged)
}

fn json_to_toml_table(v: &Json) -> toml_edit::Table {
    let mut t = toml_edit::Table::new();
    if let Json::Object(m) = v {
        for (k, v) in m {
            t.insert(k, toml_edit::Item::Value(json_to_toml_value(v)));
        }
    }
    t
}

fn json_to_toml_value(v: &Json) -> toml_edit::Value {
    match v {
        Json::String(s) => s.as_str().into(),
        Json::Bool(b) => (*b).into(),
        Json::Number(n) => n
            .as_i64()
            .map(Into::into)
            .unwrap_or_else(|| n.as_f64().unwrap_or_default().into()),
        Json::Array(a) => toml_edit::Value::Array(a.iter().map(json_to_toml_value).collect()),
        Json::Object(m) => {
            let mut t = toml_edit::InlineTable::new();
            for (k, v) in m {
                t.insert(k, json_to_toml_value(v));
            }
            toml_edit::Value::InlineTable(t)
        }
        Json::Null => "".into(),
    }
}

fn toml_to_json(item: &toml_edit::Item) -> Json {
    match item {
        toml_edit::Item::Value(v) => toml_value_to_json(v),
        toml_edit::Item::Table(t) => Json::Object(
            t.iter()
                .map(|(k, v)| (k.to_owned(), toml_to_json(v)))
                .collect(),
        ),
        toml_edit::Item::ArrayOfTables(a) => Json::Array(
            a.iter()
                .map(|t| toml_to_json(&toml_edit::Item::Table(t.clone())))
                .collect(),
        ),
        toml_edit::Item::None => Json::Null,
    }
}

fn toml_value_to_json(v: &toml_edit::Value) -> Json {
    use toml_edit::Value as V;
    match v {
        V::String(s) => json!(s.value()),
        V::Integer(i) => json!(i.value()),
        V::Float(f) => json!(f.value()),
        V::Boolean(b) => json!(b.value()),
        V::Datetime(d) => json!(d.value().to_string()),
        V::Array(a) => Json::Array(a.iter().map(toml_value_to_json).collect()),
        V::InlineTable(t) => Json::Object(
            t.iter()
                .map(|(k, v)| (k.to_owned(), toml_value_to_json(v)))
                .collect(),
        ),
    }
}

/// Reads `path`, merges, and writes it back atomically with a
/// `.loadout.bak` of the previous version. With `private`, the file is
/// written with mode `0600` (Unix) because it holds secrets.
pub fn apply(
    format: Format,
    path: &Path,
    display: &str,
    key: &str,
    desired: &BTreeMap<String, Json>,
    owned: &BTreeSet<String>,
    private: bool,
) -> Result<Merged, McpError> {
    let io = |source| McpError::Io {
        path: display.to_owned(),
        source,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io(e)),
    };
    let merged = merge(format, display, &text, key, desired, owned)?;
    if let Some(new) = &merged.text {
        atomic_write_opts(
            path,
            new.as_bytes(),
            WriteOptions {
                backup: true,
                private,
            },
        )
        .map_err(io)?;
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio() -> Delivered {
        Delivered::Stdio {
            command: "/opt/lo".into(),
            args: vec!["mcp-run".into(), "acme:mcp/jira".into()],
            env: BTreeMap::new(),
        }
    }

    fn remote() -> Delivered {
        Delivered::Remote {
            transport: Transport::Http,
            url: Value::lit("https://mcp.example.com/mcp"),
            headers: [
                (
                    "Authorization".to_string(),
                    Value(vec![
                        Piece::Lit("Bearer ".into()),
                        Piece::Env("TOKEN".into()),
                    ]),
                ),
                ("X-Team".to_string(), Value::lit("payments")),
            ]
            .into(),
            headers_helper: None,
        }
    }

    #[test]
    fn renders_each_format() {
        let all = [
            Format::ClaudeCodeJson,
            Format::CursorJson,
            Format::PiMcpJson,
            Format::OpencodeJson,
            Format::CodexToml,
            Format::BobJson,
        ];
        let out: Vec<String> = all
            .iter()
            .flat_map(|f| {
                [
                    format!("{f:?} stdio: {}", f.render(&stdio())),
                    format!("{f:?} remote: {}", f.render(&remote())),
                ]
            })
            .collect();
        insta::assert_snapshot!(out.join("\n"));
    }

    #[test]
    fn renders_headers_helper_and_sse_for_claude() {
        let d = Delivered::Remote {
            transport: Transport::Sse,
            url: Value::lit("https://x.example.com/sse"),
            headers: BTreeMap::new(),
            headers_helper: Some("/opt/lo mcp-headers acme:mcp/x".into()),
        };
        assert_eq!(
            Format::ClaudeCodeJson.render(&d),
            json!({"type": "sse", "url": "https://x.example.com/sse", "headersHelper": "/opt/lo mcp-headers acme:mcp/x"})
        );
    }

    fn desired(pairs: &[(&str, Json)]) -> BTreeMap<String, Json> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn owned(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    const CLAUDE_JSON: &str = r#"{
  "numStartups": 42,
  "mcpServers": {
    "mine": {
      "type": "stdio",
      "command": "my-server"
    }
  },
  "projects": {}
}
"#;

    #[test]
    fn json_merge_preserves_everything_else() {
        let d = desired(&[("jira", json!({"command": "x"}))]);
        let m = merge(
            Format::ClaudeCodeJson,
            "~/.claude.json",
            CLAUDE_JSON,
            "mcpServers",
            &d,
            &owned(&[]),
        )
        .unwrap();
        assert_eq!(m.added, ["jira"]);
        let text = m.text.unwrap();
        insta::assert_snapshot!(text);
        // Re-merging is a no-op.
        let again = merge(
            Format::ClaudeCodeJson,
            "~/.claude.json",
            &text,
            "mcpServers",
            &d,
            &m.owned,
        )
        .unwrap();
        assert_eq!(again.text, None);
        assert_eq!(again.unchanged, 1);
        // Removing: only our entry goes.
        let gone = merge(
            Format::ClaudeCodeJson,
            "~/.claude.json",
            &text,
            "mcpServers",
            &BTreeMap::new(),
            &m.owned,
        )
        .unwrap();
        assert_eq!(gone.removed, ["jira"]);
        assert_eq!(gone.text.unwrap(), CLAUDE_JSON);
    }

    #[test]
    fn json_collision_and_adoption() {
        let d = desired(&[("mine", json!({"command": "other"}))]);
        let m = merge(
            Format::ClaudeCodeJson,
            "f",
            CLAUDE_JSON,
            "mcpServers",
            &d,
            &owned(&[]),
        )
        .unwrap();
        assert_eq!(m.collisions, ["mine"]);
        assert!(m.text.is_none());
        assert!(m.owned.is_empty());

        let d = desired(&[("mine", json!({"type": "stdio", "command": "my-server"}))]);
        let m = merge(
            Format::ClaudeCodeJson,
            "f",
            CLAUDE_JSON,
            "mcpServers",
            &d,
            &owned(&[]),
        )
        .unwrap();
        assert_eq!(m.unchanged, 1);
        assert_eq!(m.owned, owned(&["mine"]));
    }

    #[test]
    fn json_update_and_new_file() {
        let d = desired(&[("jira", json!({"command": "v2"}))]);
        let m = merge(Format::CursorJson, "f", "", "mcpServers", &d, &owned(&[])).unwrap();
        assert_eq!(
            m.text.as_deref(),
            Some(
                "{\n  \"mcpServers\": {\n    \"jira\": {\n      \"command\": \"v2\"\n    }\n  }\n}\n"
            )
        );
        let d2 = desired(&[("jira", json!({"command": "v3"}))]);
        let m2 = merge(
            Format::CursorJson,
            "f",
            m.text.as_deref().unwrap(),
            "mcpServers",
            &d2,
            &m.owned,
        )
        .unwrap();
        assert_eq!(m2.updated, ["jira"]);
    }

    #[test]
    fn json_errors_leave_file_alone() {
        let d = desired(&[("jira", json!({}))]);
        let e = merge(
            Format::OpencodeJson,
            "~/.config/opencode/opencode.json",
            "{ // comment\n}",
            "mcp",
            &d,
            &owned(&[]),
        )
        .unwrap_err();
        assert!(e.to_string().contains("not valid JSON"), "{e}");
        let e = merge(
            Format::OpencodeJson,
            "f",
            "{\"mcp\": []}",
            "mcp",
            &d,
            &owned(&[]),
        )
        .unwrap_err();
        assert!(e.to_string().contains("`mcp` is not"), "{e}");
        let e = merge(Format::OpencodeJson, "f", "[]", "mcp", &d, &owned(&[])).unwrap_err();
        assert!(e.to_string().contains("(root)"), "{e}");
    }

    const CODEX: &str = r#"# my codex config
model = "gpt-5"

[mcp_servers.mine]
command = "my-server" # keep this comment
"#;

    #[test]
    fn toml_merge_preserves_comments() {
        let d = desired(&[("jira", Format::CodexToml.render(&remote()))]);
        let m = merge(
            Format::CodexToml,
            "~/.codex/config.toml",
            CODEX,
            "mcp_servers",
            &d,
            &owned(&[]),
        )
        .unwrap();
        let text = m.text.unwrap();
        insta::assert_snapshot!(text);
        let again = merge(Format::CodexToml, "f", &text, "mcp_servers", &d, &m.owned).unwrap();
        assert_eq!(again.text, None, "re-merge must be a no-op");
        let gone = merge(
            Format::CodexToml,
            "f",
            &text,
            "mcp_servers",
            &BTreeMap::new(),
            &m.owned,
        )
        .unwrap();
        assert_eq!(gone.text.unwrap(), CODEX);
        let clash = desired(&[("mine", json!({"command": "x"}))]);
        let m = merge(
            Format::CodexToml,
            "f",
            CODEX,
            "mcp_servers",
            &clash,
            &owned(&[]),
        )
        .unwrap();
        assert_eq!(m.collisions, ["mine"]);
    }

    #[test]
    fn apply_writes_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        std::fs::write(&path, CLAUDE_JSON).unwrap();
        let d = desired(&[("jira", json!({"command": "x"}))]);
        let m = apply(
            Format::ClaudeCodeJson,
            &path,
            "~/.claude.json",
            "mcpServers",
            &d,
            &owned(&[]),
            false,
        )
        .unwrap();
        assert_eq!(m.added, ["jira"]);
        assert_eq!(
            std::fs::read_to_string(crate::fsutil::backup_path(&path)).unwrap(),
            CLAUDE_JSON
        );
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"jira\""));
    }

    #[test]
    fn unknown_renderer() {
        assert!(Format::from_renderer("claude-code-json").is_ok());
        assert!(Format::from_renderer("bob-json").is_ok());
        assert!(Format::from_renderer("nope").is_err());
    }
}

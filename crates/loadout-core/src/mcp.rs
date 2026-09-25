//! How an MCP server reaches a target without leaking secrets
//!: a pure decision per server and target.
//!
//! In order of preference:
//! 1. stdio servers with references launch through `lo mcp-run <id>`,
//!    which resolves them at launch;
//! 2. remote servers use a headers helper (`lo mcp-headers <id>`) when
//!    the target has one, or the target's own `${VAR}` expansion for
//!    `env://` references;
//! 3. with `allow_secrets_on_disk`, the resolved value is written into the
//!    target config (last resort);
//! 4. otherwise the server is skipped with a reason.

use std::collections::BTreeMap;

use loadout_model::secret::{Part, Template};
use loadout_model::{ItemId, McpServer, SecretRef, Transport};

/// What a target's MCP config format can express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Caps {
    /// How the target expands environment variables in remote `url` and
    /// `headers`.
    pub env: EnvSupport,
    /// The target can run a command that prints extra headers (Claude
    /// Code's `headersHelper`).
    pub headers_helper: bool,
    /// The target supports the SSE transport.
    pub sse: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvSupport {
    #[default]
    None,
    /// `${VAR}`-style interpolation anywhere in `url` and header values.
    Interpolate,
    /// Only a whole header value (or `Authorization: Bearer <var>`) can
    /// come from a variable (Codex).
    WholeHeader,
}

/// A rendered string: literal text, environment references the target
/// expands itself, and (only when writing secrets to disk is allowed)
/// secret references the caller must resolve before writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Lit(String),
    Env(String),
    Secret(SecretRef),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Value(pub Vec<Piece>);

impl Value {
    pub fn lit(s: impl Into<String>) -> Self {
        Value(vec![Piece::Lit(s.into())])
    }

    /// The literal text, if the value has no references.
    pub fn as_lit(&self) -> Option<String> {
        self.0
            .iter()
            .map(|p| match p {
                Piece::Lit(l) => Some(l.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn has_secrets(&self) -> bool {
        self.0.iter().any(|p| matches!(p, Piece::Secret(_)))
    }
}

/// A server in a target-neutral shape, ready for a renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivered {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    Remote {
        transport: Transport,
        url: Value,
        headers: BTreeMap<String, Value>,
        /// Command printing the headers left out of `headers`.
        headers_helper: Option<String>,
    },
}

/// The outcome for one server in one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    Install {
        server: Delivered,
        /// True when `server` contains [`Piece::Secret`]s that must be
        /// resolved and written to disk.
        secrets_on_disk: bool,
    },
    Skip(String),
}

pub struct Input<'a> {
    pub id: &'a ItemId,
    pub server: &'a McpServer,
    pub caps: Caps,
    /// Absolute path of the `lo` binary, used for `mcp-run` and
    /// `mcp-headers`.
    pub loadout_bin: &'a str,
    /// Global arguments placed before `mcp-run` / `mcp-headers` (e.g.
    /// `--project-dir <root>` for a project's servers).
    pub loadout_args: &'a [String],
    pub allow_secrets_on_disk: bool,
}

pub fn plan(input: &Input<'_>) -> Plan {
    match input.server {
        McpServer::Stdio { command, args, env } => {
            let direct = std::iter::once(command)
                .chain(args)
                .chain(env.values())
                .all(|t| !t.has_refs());
            let server = if direct {
                Delivered::Stdio {
                    command: command.to_string(),
                    args: args.iter().map(ToString::to_string).collect(),
                    env: env
                        .iter()
                        .map(|(k, v)| (k.clone(), v.to_string()))
                        .collect(),
                }
            } else {
                let mut args = input.loadout_args.to_vec();
                args.extend(["mcp-run".to_owned(), input.id.to_string()]);
                Delivered::Stdio {
                    command: input.loadout_bin.to_owned(),
                    args,
                    env: BTreeMap::new(),
                }
            };
            Plan::Install {
                server,
                secrets_on_disk: false,
            }
        }
        McpServer::Remote {
            transport,
            url,
            headers,
        } => plan_remote(input, *transport, url, headers),
    }
}

fn plan_remote(
    input: &Input<'_>,
    transport: Transport,
    url: &Template,
    headers: &BTreeMap<String, Template>,
) -> Plan {
    let caps = input.caps;
    if transport == Transport::Sse && !caps.sse {
        return Plan::Skip("this target does not support the SSE transport".into());
    }
    let mut on_disk = false;
    let mut blocked: Vec<String> = Vec::new();

    let url_value = match expand(url, caps.env == EnvSupport::Interpolate) {
        Some(v) => v,
        None if input.allow_secrets_on_disk => {
            on_disk = true;
            materialize(url)
        }
        None => {
            blocked.push("url".into());
            Value::default()
        }
    };

    let mut out_headers = BTreeMap::new();
    let mut helper = false;
    for (name, t) in headers {
        if !t.has_refs() {
            out_headers.insert(name.clone(), Value::lit(t.to_string()));
            continue;
        }
        if caps.headers_helper {
            helper = true;
            continue;
        }
        let env_ok = match caps.env {
            EnvSupport::Interpolate => true,
            EnvSupport::WholeHeader => whole_header_env(name, t),
            EnvSupport::None => false,
        };
        match expand(t, env_ok) {
            Some(v) => {
                out_headers.insert(name.clone(), v);
            }
            None if input.allow_secrets_on_disk => {
                on_disk = true;
                out_headers.insert(name.clone(), materialize(t));
            }
            None => blocked.push(format!("header {name}")),
        }
    }

    if !blocked.is_empty() {
        return Plan::Skip(format!(
            "{} {} secret references this target cannot receive without writing them to disk \
             (set `allow_secrets_on_disk = true` under [secrets] to allow that)",
            blocked.join(", "),
            if blocked.len() == 1 {
                "contains"
            } else {
                "contain"
            },
        ));
    }
    Plan::Install {
        server: Delivered::Remote {
            transport,
            url: url_value,
            headers: out_headers,
            headers_helper: helper.then(|| {
                let mut cmd = shell_quote(input.loadout_bin);
                for a in input.loadout_args {
                    cmd.push(' ');
                    cmd.push_str(&shell_quote(a));
                }
                format!("{cmd} mcp-headers {}", input.id)
            }),
        },
        secrets_on_disk: on_disk,
    }
}

/// Renders `t` with `env://` references as [`Piece::Env`] when `env_ok`;
/// `None` if it contains references that cannot be expressed that way.
fn expand(t: &Template, env_ok: bool) -> Option<Value> {
    let mut out = Vec::new();
    for p in &t.parts {
        match p {
            Part::Lit(l) => out.push(Piece::Lit(l.clone())),
            Part::Ref(SecretRef::Env(v)) if env_ok => out.push(Piece::Env(v.clone())),
            Part::Ref(_) => return None,
        }
    }
    Some(Value(out))
}

fn materialize(t: &Template) -> Value {
    Value(
        t.parts
            .iter()
            .map(|p| match p {
                Part::Lit(l) => Piece::Lit(l.clone()),
                Part::Ref(r) => Piece::Secret(r.clone()),
            })
            .collect(),
    )
}

/// Codex can take a header from a variable only as the whole value, or as
/// the token of `Authorization: Bearer <token>`.
fn whole_header_env(name: &str, t: &Template) -> bool {
    match t.parts.as_slice() {
        [Part::Ref(SecretRef::Env(_))] => true,
        [Part::Lit(prefix), Part::Ref(SecretRef::Env(_))] => {
            name.eq_ignore_ascii_case("authorization") && prefix == "Bearer "
        }
        _ => false,
    }
}

/// Quotes a path for the shell that runs a headers helper: double quotes,
/// which both `sh` and `cmd.exe` understand for ordinary paths.
pub fn shell_quote(path: &str) -> String {
    if path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/\\._-:".contains(&b))
    {
        path.to_owned()
    } else {
        format!("\"{}\"", path.replace('"', "\\\""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loadout_model::frontmatter::Document;
    use loadout_model::{McpFrontmatter, McpServer};

    fn server(fm: &str) -> McpServer {
        let fm: McpFrontmatter = Document::split(&format!("---\n{fm}\n---\n"))
            .unwrap()
            .parse()
            .unwrap();
        McpServer::from_frontmatter(&fm).unwrap()
    }

    fn run(fm: &str, caps: Caps, allow: bool) -> Plan {
        let id: ItemId = "acme:mcp/jira".parse().unwrap();
        let s = server(fm);
        plan(&Input {
            id: &id,
            server: &s,
            caps,
            loadout_bin: "/usr/local/bin/lo",
            loadout_args: &[],
            allow_secrets_on_disk: allow,
        })
    }

    const CLAUDE: Caps = Caps {
        env: EnvSupport::Interpolate,
        headers_helper: true,
        sse: true,
    };
    const CODEX: Caps = Caps {
        env: EnvSupport::WholeHeader,
        headers_helper: false,
        sse: false,
    };
    const NONE: Caps = Caps {
        env: EnvSupport::None,
        headers_helper: false,
        sse: true,
    };

    #[test]
    fn stdio_without_refs_is_direct() {
        let p = run("command: npx\nargs: [-y, x]\nenv: { A: b }", NONE, false);
        assert_eq!(
            p,
            Plan::Install {
                server: Delivered::Stdio {
                    command: "npx".into(),
                    args: vec!["-y".into(), "x".into()],
                    env: [("A".to_string(), "b".to_string())].into(),
                },
                secrets_on_disk: false
            }
        );
    }

    #[test]
    fn stdio_with_refs_uses_wrapper() {
        let p = run(
            "command: npx\nenv: { T: \"secret://jira/token\" }",
            NONE,
            true,
        );
        assert_eq!(
            p,
            Plan::Install {
                server: Delivered::Stdio {
                    command: "/usr/local/bin/lo".into(),
                    args: vec!["mcp-run".into(), "acme:mcp/jira".into()],
                    env: BTreeMap::new(),
                },
                secrets_on_disk: false
            }
        );
    }

    #[test]
    fn remote_headers_use_helper_when_available() {
        let p = run(
            "url: https://mcp.example.com\nheaders: { Authorization: \"Bearer secret://jira/token\", X-Team: payments }",
            CLAUDE,
            false,
        );
        let Plan::Install {
            server:
                Delivered::Remote {
                    headers,
                    headers_helper,
                    ..
                },
            secrets_on_disk: false,
        } = p
        else {
            panic!("{p:?}")
        };
        assert_eq!(headers.len(), 1);
        assert_eq!(headers["X-Team"], Value::lit("payments"));
        assert_eq!(
            headers_helper.as_deref(),
            Some("/usr/local/bin/lo mcp-headers acme:mcp/jira")
        );
    }

    #[test]
    fn env_refs_use_target_expansion() {
        let p = run(
            "url: https://x.example.com\nheaders: { Authorization: \"Bearer env://TOKEN\" }",
            Caps {
                headers_helper: false,
                ..CLAUDE
            },
            false,
        );
        let Plan::Install {
            server: Delivered::Remote { headers, .. },
            ..
        } = p
        else {
            panic!("{p:?}")
        };
        assert_eq!(
            headers["Authorization"],
            Value(vec![
                Piece::Lit("Bearer ".into()),
                Piece::Env("TOKEN".into())
            ])
        );
    }

    #[test]
    fn codex_takes_whole_or_bearer_env_headers_only() {
        let ok = run(
            "url: https://x.example.com\nheaders: { Authorization: \"Bearer env://T\", X-Key: \"env://K\" }",
            CODEX,
            false,
        );
        assert!(matches!(ok, Plan::Install { .. }), "{ok:?}");
        let bad = run(
            "url: https://x.example.com\nheaders: { X-Key: \"key=env://K\" }",
            CODEX,
            false,
        );
        assert!(
            matches!(bad, Plan::Skip(ref m) if m.contains("header X-Key")),
            "{bad:?}"
        );
    }

    #[test]
    fn secrets_go_to_disk_only_when_allowed() {
        let fm =
            "url: https://x.example.com/secret://tenant/id\nheaders: { X-Key: \"secret://k\" }";
        let skip = run(fm, NONE, false);
        assert!(
            matches!(skip, Plan::Skip(ref m) if m.starts_with("url, header X-Key contain")),
            "{skip:?}"
        );
        let Plan::Install {
            server: Delivered::Remote { url, headers, .. },
            secrets_on_disk: true,
        } = run(fm, NONE, true)
        else {
            panic!()
        };
        assert!(url.has_secrets());
        assert!(headers["X-Key"].has_secrets());
    }

    #[test]
    fn sse_needs_support() {
        let p = run(
            "transport: sse\nurl: https://x.example.com/sse",
            CODEX,
            false,
        );
        assert!(matches!(p, Plan::Skip(ref m) if m.contains("SSE")));
    }

    #[test]
    fn quotes_paths_with_spaces() {
        assert_eq!(shell_quote("/usr/bin/lo"), "/usr/bin/lo");
        assert_eq!(
            shell_quote(r"C:\Program Files\lo.exe"),
            r#""C:\Program Files\lo.exe""#
        );
    }
}

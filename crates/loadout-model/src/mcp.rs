//! MCP server items: `mcp/<name>.md`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ModelError;
use crate::secret::{SecretRef, Template, is_env_name};

/// How the agent talks to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Stdio,
    Http,
    Sse,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Stdio => "stdio",
            Transport::Http => "http",
            Transport::Sse => "sse",
        }
    }
}

/// The server-specific frontmatter fields of an MCP item, as written.
/// `name`, `description` and `loadout:` are read through
/// [`ItemMeta`](crate::ItemMeta); other unknown keys are ignored.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct McpFrontmatter {
    /// Must be `mcp` when present.
    pub kind: Option<String>,
    /// Defaults to `stdio` when `command` is set, else `http`.
    pub transport: Option<Transport>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub url: Option<String>,
    pub headers: BTreeMap<String, String>,
}

/// A validated MCP server definition. String values may contain secret
/// references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServer {
    Stdio {
        command: Template,
        args: Vec<Template>,
        env: BTreeMap<String, Template>,
    },
    /// `http` or `sse`.
    Remote {
        transport: Transport,
        url: Template,
        headers: BTreeMap<String, Template>,
    },
}

impl McpServer {
    /// Validates frontmatter into a server definition.
    pub fn from_frontmatter(fm: &McpFrontmatter) -> Result<Self, ModelError> {
        let err = |m: &str| Err(ModelError::InvalidMcp(m.to_owned()));
        if let Some(kind) = fm.kind.as_deref()
            && kind != "mcp"
        {
            return err(&format!("kind must be \"mcp\", got {kind:?}"));
        }
        let transport = match (fm.transport, &fm.command, &fm.url) {
            (Some(t), _, _) => t,
            (None, Some(_), _) => Transport::Stdio,
            (None, None, Some(_)) => Transport::Http,
            (None, None, None) => return err("set `command` (stdio) or `url` (http/sse)"),
        };
        match transport {
            Transport::Stdio => {
                let Some(command) = fm.command.as_deref().filter(|c| !c.trim().is_empty()) else {
                    return err("stdio servers need a `command`");
                };
                if fm.url.is_some() || !fm.headers.is_empty() {
                    return err(
                        "stdio servers take `command`, `args` and `env`, not `url`/`headers`",
                    );
                }
                for k in fm.env.keys() {
                    if !is_env_name(k) {
                        return err(&format!("invalid environment variable name {k:?}"));
                    }
                }
                Ok(McpServer::Stdio {
                    command: Template::parse(command)?,
                    args: fm
                        .args
                        .iter()
                        .map(|a| Template::parse(a))
                        .collect::<Result<_, _>>()?,
                    env: fm
                        .env
                        .iter()
                        .map(|(k, v)| Ok((k.clone(), Template::parse(v)?)))
                        .collect::<Result<_, ModelError>>()?,
                })
            }
            Transport::Http | Transport::Sse => {
                let Some(url) = fm.url.as_deref().filter(|u| !u.trim().is_empty()) else {
                    return err(&format!("{} servers need a `url`", transport.as_str()));
                };
                if fm.command.is_some() || !fm.args.is_empty() || !fm.env.is_empty() {
                    return err(&format!(
                        "{} servers take `url` and `headers`, not `command`/`args`/`env`",
                        transport.as_str()
                    ));
                }
                for k in fm.headers.keys() {
                    let ok = !k.is_empty()
                        && k.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
                    if !ok {
                        return err(&format!("invalid header name {k:?}"));
                    }
                }
                Ok(McpServer::Remote {
                    transport,
                    url: Template::parse(url)?,
                    headers: fm
                        .headers
                        .iter()
                        .map(|(k, v)| Ok((k.clone(), Template::parse(v)?)))
                        .collect::<Result<_, ModelError>>()?,
                })
            }
        }
    }

    pub fn transport(&self) -> Transport {
        match self {
            McpServer::Stdio { .. } => Transport::Stdio,
            McpServer::Remote { transport, .. } => *transport,
        }
    }

    /// Every secret reference in the definition, deduplicated and sorted.
    pub fn refs(&self) -> Vec<SecretRef> {
        let mut out: Vec<SecretRef> = match self {
            McpServer::Stdio { command, args, env } => std::iter::once(command)
                .chain(args)
                .chain(env.values())
                .flat_map(Template::refs)
                .cloned()
                .collect(),
            McpServer::Remote { url, headers, .. } => std::iter::once(url)
                .chain(headers.values())
                .flat_map(Template::refs)
                .cloned()
                .collect(),
        };
        out.sort();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::Document;

    fn parse(text: &str) -> Result<McpServer, ModelError> {
        let fm: McpFrontmatter = Document::split(text)?.parse()?;
        McpServer::from_frontmatter(&fm)
    }

    const JIRA: &str = r#"---
name: jira
kind: mcp
loadout: { mode: required, tags: [jira, tickets] }
transport: stdio
command: npx
args: ["-y", "@acme/jira-mcp"]
env:
  JIRA_BASE_URL: "https://acme.atlassian.net"
  JIRA_TOKEN: "secret://jira/token"
---
Connects the agent to Jira using Acme's ticket conventions.
"#;

    #[test]
    fn parses_stdio_example() {
        let s = parse(JIRA).unwrap();
        let McpServer::Stdio { command, args, env } = &s else {
            panic!("{s:?}")
        };
        assert_eq!(command.literal().as_deref(), Some("npx"));
        assert_eq!(args.len(), 2);
        assert_eq!(
            env["JIRA_BASE_URL"].literal().as_deref(),
            Some("https://acme.atlassian.net")
        );
        assert_eq!(s.refs(), vec![SecretRef::Named("jira/token".into())]);
    }

    #[test]
    fn parses_http_with_embedded_ref_and_infers_transport() {
        let s = parse(
            "---\nurl: https://mcp.example.com/mcp\nheaders: { Authorization: \"Bearer secret://jira/token\" }\n---\n",
        )
        .unwrap();
        assert_eq!(s.transport(), Transport::Http);
        assert_eq!(s.refs().len(), 1);
        let s = parse("---\ntransport: sse\nurl: https://x.example.com/sse\n---\n").unwrap();
        assert_eq!(s.transport(), Transport::Sse);
        assert_eq!(
            parse("---\ncommand: x\n---\n").unwrap().transport(),
            Transport::Stdio
        );
    }

    #[test]
    fn rejects_invalid_definitions() {
        for (text, needle) in [
            ("---\nkind: skill\ncommand: x\n---\n", "kind"),
            ("---\ndescription: nothing\n---\n", "command"),
            (
                "---\ntransport: stdio\nurl: https://x\n---\n",
                "need a `command`",
            ),
            ("---\ncommand: x\nurl: https://x\n---\n", "not `url`"),
            ("---\ntransport: http\ncommand: x\n---\n", "need a `url`"),
            ("---\nurl: https://x\nenv: { A: b }\n---\n", "not `command`"),
            (
                "---\ncommand: x\nenv: { \"A-B\": c }\n---\n",
                "variable name",
            ),
            (
                "---\nurl: https://x\nheaders: { \"Bad Header\": c }\n---\n",
                "header name",
            ),
            (
                "---\ncommand: x\nenv: { A: \"env://1x\" }\n---\n",
                "secret reference",
            ),
            ("---\ntransport: websocket\nurl: wss://x\n---\n", "YAML"),
        ] {
            let e = parse(text).unwrap_err().to_string();
            assert!(e.contains(needle), "{text}: {e}");
        }
    }
}

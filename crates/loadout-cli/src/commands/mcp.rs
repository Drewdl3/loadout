//! `lo mcp-run <id>` and `lo mcp-headers <id>`: launch-time secret
//! delivery for MCP servers. Target configs point at these
//! instead of holding secret values.
//!
//! Both are called by AI tools, not people. `mcp-run`'s stdout belongs to
//! the MCP protocol, so everything Loadout says goes to stderr.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Args;
use loadout_git::Git;
use loadout_model::config::normalize_url;
use loadout_model::frontmatter::Document;
use loadout_model::{ItemId, ItemKind, McpFrontmatter, McpServer, Template};
use loadout_secrets::{ExposeSecret, SecretResolver};

use crate::ctx::Ctx;
use crate::membership;
use crate::scan::mcp_file_name;
use crate::secrets::{self, System};
use crate::state::Resolved;
use crate::store;

#[derive(Debug, Args)]
pub struct McpArgs {
    /// The MCP item, `source:mcp/name`.
    pub id: ItemId,
}

/// An installed MCP item, read back from the store.
pub struct Installed {
    pub id: ItemId,
    pub server: McpServer,
    /// The checkout of the item's source (for `sops://`).
    pub source_dir: Option<PathBuf>,
}

/// Loads `id` from the store (it is there iff the last sync installed it).
pub fn load_installed(ctx: &Ctx, id: &ItemId) -> Result<Installed> {
    if id.key().kind() != ItemKind::Mcp {
        bail!("{id} is not an MCP item");
    }
    let path = ctx
        .paths
        .store_dir()
        .join(store::item_rel_path(id))
        .join(mcp_file_name(id.key().name()));
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "{id} is not installed (no {}); run `lo sync`",
            path.display()
        )
    })?;
    let doc = Document::split(&text)?;
    let server = McpServer::from_frontmatter(&doc.parse::<McpFrontmatter>()?)
        .with_context(|| format!("{id}"))?;
    let source_dir = Resolved::load(&ctx.paths.resolved_file())?.and_then(|r| {
        r.sources
            .iter()
            .find(|s| s.name == id.source())
            .map(|s| loadout_git::repo_dir(&ctx.paths.repos_dir(), &normalize_url(&s.url)))
    });
    Ok(Installed {
        id: id.clone(),
        server,
        source_dir,
    })
}

/// The real resolvers with the configured chain. The company config is read from
/// its local checkout (never fetched: this runs on every server launch).
pub fn system(ctx: &Ctx) -> Result<System> {
    let config = ctx.load_config()?;
    let company_config = match &config.company_config {
        Some(url) => match membership::load_company_config(ctx, &Git::new(), url, false) {
            Ok(c) => Some(c.company_config),
            Err(e) => {
                eprintln!("loadout: warning: {e:#}; using the local secret chain");
                None
            }
        },
        None => None,
    };
    Ok(System::new(secrets::chain(
        &config,
        company_config.as_ref(),
    )?))
}

fn render(t: &Template, res: &dyn SecretResolver) -> Result<String> {
    Ok(t.render(|r| res.resolve(r).map(|v| v.expose_secret().to_owned()))?)
}

/// `lo mcp-run <id>`: resolve secrets, then become the real server.
pub fn run(ctx: &Ctx, args: McpArgs) -> Result<u8> {
    let item = load_installed(ctx, &args.id)?;
    let McpServer::Stdio {
        command,
        args: argv,
        env,
    } = &item.server
    else {
        bail!("{} is not a stdio server", item.id);
    };
    let system = system(ctx)?;
    let res = system.resolvers(item.source_dir.clone());
    let program = render(command, &res)?;
    let argv: Vec<String> = argv
        .iter()
        .map(|a| render(a, &res))
        .collect::<Result<_>>()?;
    let env: BTreeMap<String, String> = env
        .iter()
        .map(|(k, v)| Ok((k.clone(), render(v, &res)?)))
        .collect::<Result<_>>()?;

    let mut cmd = std::process::Command::new(&program);
    cmd.args(&argv).envs(&env);
    exec(cmd, &program)
}

#[cfg(unix)]
fn exec(mut cmd: std::process::Command, program: &str) -> Result<u8> {
    use std::os::unix::process::CommandExt;
    // Only returns on failure.
    let e = cmd.exec();
    Err(e).with_context(|| format!("starting {program}"))
}

/// Windows has no `exec`: run the server with inherited stdio and pass its
/// exit code through.
#[cfg(not(unix))]
fn exec(mut cmd: std::process::Command, program: &str) -> Result<u8> {
    let status = cmd
        .status()
        .with_context(|| format!("starting {program}"))?;
    let code = status.code().unwrap_or(1);
    std::process::exit(code);
}

/// `lo mcp-headers <id>`: print the headers that carry secrets as a JSON
/// object (Claude Code `headersHelper`).
pub fn headers(ctx: &Ctx, args: McpArgs) -> Result<u8> {
    let item = load_installed(ctx, &args.id)?;
    let McpServer::Remote { headers, .. } = &item.server else {
        bail!("{} is not an http/sse server", item.id);
    };
    let system = system(ctx)?;
    let res = system.resolvers(item.source_dir.clone());
    let mut out = serde_json::Map::new();
    for (name, t) in headers.iter().filter(|(_, t)| t.has_refs()) {
        out.insert(name.clone(), render(t, &res)?.into());
    }
    println!("{}", serde_json::Value::Object(out));
    Ok(crate::exit::OK)
}

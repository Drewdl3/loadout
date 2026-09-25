//! `lo doctor` — diagnose the local setup.

use std::fmt::Write as _;

use anyhow::Result;
use clap::Args;
use loadout_core::links::Existing;
use loadout_git::Git;
use loadout_model::{Config, LinkMode};
use loadout_targets::owned::OwnedFile;
use loadout_targets::{TargetTable, link};
use schemars::JsonSchema;
use serde::Serialize;

use std::collections::BTreeSet;

use loadout_model::{ItemKind, SecretRef};
use loadout_secrets::Chain;

use crate::commands::mcp::load_installed;
use crate::ctx::{Ctx, Report};
use crate::engine::{display_path, enabled_targets};
use crate::exit;
use crate::state::Resolved;

#[derive(Debug, Args)]
pub struct DoctorArgs {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Check {
    /// Stable check id, e.g. `git`, `config`, `broken-link`, `collision`.
    pub check: String,
    pub status: Status,
    pub message: String,
}

/// `--json` output of `lo doctor`.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct DoctorReport {
    pub checks: Vec<Check>,
}

impl DoctorReport {
    fn push(&mut self, check: &str, status: Status, message: impl Into<String>) {
        self.checks.push(Check {
            check: check.into(),
            status,
            message: message.into(),
        });
    }
}

impl Report for DoctorReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        for c in &self.checks {
            let tag = match c.status {
                Status::Ok => "ok   ",
                Status::Warn => "warn ",
                Status::Error => "error",
            };
            writeln!(out, "{tag}  {:<12} {}", c.check, c.message)?;
        }
        let problems = self
            .checks
            .iter()
            .filter(|c| c.status != Status::Ok)
            .count();
        if problems == 0 {
            writeln!(out, "No problems found.")
        } else {
            writeln!(out, "{problems} problem(s) found.")
        }
    }
}

pub fn run(ctx: &Ctx, _args: DoctorArgs) -> Result<u8> {
    let mut r = DoctorReport::default();

    match Git::new().version() {
        Ok(v) => r.push("git", Status::Ok, v),
        Err(e) => r.push("git", Status::Error, e.to_string()),
    }

    let config = match ctx.load_config() {
        Ok(c) => {
            r.push(
                "config",
                Status::Ok,
                format!(
                    "{} ({} manual source{})",
                    display_path(&ctx.paths.config_file(), &ctx.paths.home),
                    c.sources.len(),
                    if c.sources.len() == 1 { "" } else { "s" }
                ),
            );
            c
        }
        Err(e) => {
            r.push("config", Status::Error, format!("{e:#}"));
            Config::default()
        }
    };

    let table = match TargetTable::load(&ctx.paths.user_targets_dir()) {
        Ok(t) => t,
        Err(e) => {
            r.push("targets", Status::Error, e.to_string());
            TargetTable::default()
        }
    };
    let (targets, warnings) = enabled_targets(&table, &config, &ctx.paths.home);
    for w in warnings {
        r.push("targets", Status::Warn, w);
    }
    if !targets.is_empty() {
        let ids: Vec<&str> = targets.iter().map(|t| t.id.as_str()).collect();
        r.push("targets", Status::Ok, ids.join(", "));
    }

    let owned = match OwnedFile::load(&ctx.paths.owned_file()) {
        Ok(o) => o,
        Err(e) => {
            r.push("owned", Status::Error, e.to_string());
            OwnedFile::default()
        }
    };
    check_owned(ctx, &owned, &mut r);
    for (target, m) in &owned.mcp {
        if !m.secrets_on_disk.is_empty() {
            let names: Vec<&str> = m.secrets_on_disk.iter().map(String::as_str).collect();
            r.push(
                "secret-on-disk",
                Status::Warn,
                format!(
                    "{target}: {} holds resolved secrets for MCP server(s) {} (allow_secrets_on_disk)",
                    display_path(&m.path, &ctx.paths.home),
                    names.join(", ")
                ),
            );
        }
    }

    match Resolved::load(&ctx.paths.resolved_file()) {
        Ok(Some(resolved)) => {
            if !ctx.paths.store_dir().is_dir() {
                r.push("store", Status::Error, "store is missing; run `lo sync`");
            }
            check_collisions(ctx, &resolved, &mut r);
            check_secret_tools(ctx, &config, &resolved, &mut r);
        }
        Ok(None) => r.push("sync", Status::Warn, "nothing synced yet; run `lo sync`"),
        Err(e) => r.push("sync", Status::Error, format!("{e:#}")),
    }

    let failed = r.checks.iter().any(|c| c.status == Status::Error);
    ctx.emit(&r)?;
    Ok(if failed { exit::ERROR } else { exit::OK })
}

fn check_owned(ctx: &Ctx, owned: &OwnedFile, r: &mut DoctorReport) {
    for (target, entries) in &owned.targets {
        for o in entries {
            let shown = display_path(&o.dest, &ctx.paths.home);
            match (link::inspect(&o.dest), o.mode) {
                (Existing::Missing, _) => r.push(
                    "missing",
                    Status::Warn,
                    format!("{target}: {shown} was removed; `lo sync` restores it"),
                ),
                (Existing::Link(_), LinkMode::Symlink) if !o.dest.exists() => r.push(
                    "broken-link",
                    Status::Error,
                    format!("{target}: {shown} points to a missing store entry; run `lo sync`"),
                ),
                (Existing::Link(_), LinkMode::Symlink)
                | (Existing::Dir | Existing::File, LinkMode::Copy) => {}
                (_, _) => r.push(
                    "replaced",
                    Status::Warn,
                    format!(
                        "{target}: {shown} was replaced outside Loadout; it will be left alone"
                    ),
                ),
            }
        }
    }
}

fn check_collisions(ctx: &Ctx, resolved: &Resolved, r: &mut DoctorReport) {
    for (target, placements) in &resolved.placements {
        for p in placements.iter().filter(|p| !p.installed) {
            if link::inspect(&p.dest) != Existing::Missing {
                r.push(
                    "collision",
                    Status::Warn,
                    format!(
                        "{target}: {} is not managed by Loadout, so {} is not installed",
                        display_path(&p.dest, &ctx.paths.home),
                        p.id
                    ),
                );
            }
        }
    }
}

/// Warns when installed MCP servers (or the `secret://` chain) use `op`,
/// `vault` or `sops` and the tool isn't installed.
fn check_secret_tools(ctx: &Ctx, config: &Config, resolved: &Resolved, r: &mut DoctorReport) {
    let mut schemes: BTreeSet<&'static str> = BTreeSet::new();
    let mut named = false;
    for i in resolved
        .items
        .iter()
        .filter(|i| i.enabled && i.kind == ItemKind::Mcp)
    {
        if let Ok(item) = load_installed(ctx, &i.id) {
            for rf in item.server.refs() {
                named |= matches!(rf, SecretRef::Named(_));
                schemes.insert(rf.scheme());
            }
        }
    }
    if named {
        let company_config = config.company_config.as_ref().and_then(|url| {
            crate::membership::load_company_config(ctx, &Git::new(), url, false).ok()
        });
        match crate::secrets::chain(config, company_config.as_ref().map(|c| &c.company_config)) {
            Ok(chain) => {
                for e in &chain.0 {
                    schemes.insert(Chain::candidate(e, "x").scheme());
                }
            }
            Err(e) => r.push("secrets", Status::Error, format!("{e:#}")),
        }
    }
    for (scheme, tool) in [("op", "op"), ("vault", "vault"), ("sops", "sops")] {
        if !schemes.contains(scheme) {
            continue;
        }
        let found = std::process::Command::new(tool)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success());
        if found {
            r.push(
                "tool",
                Status::Ok,
                format!("{tool} found (used by {scheme}:// references)"),
            );
        } else {
            r.push(
                "tool",
                Status::Warn,
                format!("{tool} is not installed, but {scheme}:// secret references need it"),
            );
        }
    }
}

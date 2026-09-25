//! `lo secrets check|set|clear`: verify that every secret
//! reference of the installed MCP servers resolves, and manage
//! `secret://` values in the OS keychain. Values are never printed.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{IsTerminal, Read};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use loadout_model::{ItemId, ItemKind, SecretRef};
use loadout_secrets::chain::ChainEntry;
use loadout_secrets::{KEYCHAIN_SERVICE, SecretError, SecretResolver, SecretString};
use schemars::JsonSchema;
use serde::Serialize;

use crate::commands::mcp::{load_installed, system};
use crate::ctx::{Ctx, Report};
use crate::exit;
use crate::state::Resolved;

#[derive(Debug, Args)]
pub struct SecretsArgs {
    #[command(subcommand)]
    pub command: Option<SecretsCommand>,
}

#[derive(Debug, Subcommand)]
pub enum SecretsCommand {
    /// Check that every secret reference of the installed MCP servers
    /// resolves (the default). Interactive runs offer to store missing
    /// `secret://` values in the keychain.
    Check,
    /// Store the value of `secret://<name>` in the OS keychain. Reads the
    /// value from stdin when it isn't a terminal.
    Set(NameArgs),
    /// Remove `secret://<name>` from the OS keychain.
    Clear(NameArgs),
}

#[derive(Debug, Args)]
pub struct NameArgs {
    /// Secret name, as in `secret://<name>`.
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RefStatus {
    Ok,
    Missing,
    Error,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RefCheck {
    /// The reference, e.g. `secret://jira/token`.
    pub reference: String,
    /// Items using it.
    pub items: Vec<String>,
    pub status: RefStatus,
    /// Why it didn't resolve (never the value).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// `--json` output of `lo secrets check`.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct SecretsCheckReport {
    pub references: Vec<RefCheck>,
    pub warnings: Vec<String>,
}

impl Report for SecretsCheckReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if self.references.is_empty() {
            return writeln!(out, "No secret references in installed MCP servers.");
        }
        for r in &self.references {
            let tag = match r.status {
                RefStatus::Ok => "ok     ",
                RefStatus::Missing => "missing",
                RefStatus::Error => "error  ",
            };
            writeln!(out, "{tag}  {}  ({})", r.reference, r.items.join(", "))?;
            if let Some(d) = &r.detail {
                writeln!(out, "         {d}")?;
            }
        }
        let bad = self
            .references
            .iter()
            .filter(|r| r.status != RefStatus::Ok)
            .count();
        if bad == 0 {
            writeln!(out, "All {} reference(s) resolve.", self.references.len())
        } else {
            writeln!(
                out,
                "{bad} of {} reference(s) do not resolve.",
                self.references.len()
            )
        }
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// `--json` output of `lo secrets set` / `clear`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SecretChangeReport {
    pub reference: String,
    /// `set`: always true. `clear`: whether an entry existed.
    pub changed: bool,
}

impl Report for SecretChangeReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if self.changed {
            writeln!(out, "Updated {} in the keychain.", self.reference)
        } else {
            writeln!(out, "{} was not in the keychain.", self.reference)
        }
    }
}

pub fn run(ctx: &Ctx, args: SecretsArgs) -> Result<u8> {
    match args.command.unwrap_or(SecretsCommand::Check) {
        SecretsCommand::Check => check(ctx),
        SecretsCommand::Set(a) => set(ctx, &a.name),
        SecretsCommand::Clear(a) => clear(ctx, &a.name),
    }
}

fn check(ctx: &Ctx) -> Result<u8> {
    let mut report = SecretsCheckReport::default();
    let Some(resolved) = Resolved::load(&ctx.paths.resolved_file())? else {
        bail!("nothing synced yet; run `lo sync`");
    };
    // Every enabled MCP item that may be installed in some target.
    let ids: BTreeSet<ItemId> = resolved
        .placements
        .values()
        .flatten()
        .map(|p| p.id.clone())
        .chain(
            resolved
                .items
                .iter()
                .filter(|i| i.enabled)
                .map(|i| i.id.clone()),
        )
        .filter(|id| id.key().kind() == ItemKind::Mcp)
        .collect();
    let system = system(ctx)?;
    let interactive =
        !ctx.json && std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let offers_keychain = system.chain.0.contains(&ChainEntry::Keychain);

    let mut by_ref: BTreeMap<SecretRef, (Vec<String>, Option<std::path::PathBuf>)> =
        BTreeMap::new();
    for id in &ids {
        match load_installed(ctx, id) {
            Ok(item) => {
                for r in item.server.refs() {
                    let e = by_ref
                        .entry(r)
                        .or_insert_with(|| (Vec::new(), item.source_dir.clone()));
                    e.0.push(id.to_string());
                }
            }
            Err(e) => report.warnings.push(format!("{e:#}")),
        }
    }
    for (r, (items, source_dir)) in by_ref {
        let res = system.resolvers(source_dir);
        let mut result = res.resolve(&r);
        if let (Err(e), SecretRef::Named(name)) = (&result, &r)
            && e.is_not_found()
            && interactive
            && offers_keychain
            && let Some(value) = prompt(&format!("Value for {r} (stored in the keychain)"))?
        {
            system
                .keystore
                .set(KEYCHAIN_SERVICE, name, &value)
                .map_err(anyhow::Error::msg)?;
            result = res.resolve(&r);
        }
        let (status, detail) = match result {
            Ok(_) => (RefStatus::Ok, None),
            Err(e @ SecretError::NotFound { .. }) => (RefStatus::Missing, Some(e.to_string())),
            Err(e) => (RefStatus::Error, Some(e.to_string())),
        };
        report.references.push(RefCheck {
            reference: r.to_string(),
            items,
            status,
            detail,
        });
    }
    ctx.emit(&report)?;
    Ok(
        if report.references.iter().all(|r| r.status == RefStatus::Ok) {
            exit::OK
        } else {
            exit::ERROR
        },
    )
}

fn named(name: &str) -> Result<SecretRef> {
    let r: SecretRef = format!("secret://{name}")
        .parse()
        .with_context(|| format!("invalid secret name {name:?}"))?;
    Ok(r)
}

fn set(ctx: &Ctx, name: &str) -> Result<u8> {
    let r = named(name)?;
    let value = if std::io::stdin().is_terminal() {
        prompt(&format!("Value for {r}"))?.context("no value entered")?
    } else {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .context("reading the value from stdin")?;
        let v = s.strip_suffix('\n').unwrap_or(&s);
        let v = v.strip_suffix('\r').unwrap_or(v);
        if v.is_empty() {
            bail!("empty value on stdin");
        }
        SecretString::from(v.to_owned())
    };
    loadout_secrets::default_keystore()
        .set(KEYCHAIN_SERVICE, name, &value)
        .map_err(anyhow::Error::msg)?;
    ctx.emit(&SecretChangeReport {
        reference: r.to_string(),
        changed: true,
    })?;
    Ok(exit::OK)
}

fn clear(ctx: &Ctx, name: &str) -> Result<u8> {
    let r = named(name)?;
    let existed = loadout_secrets::default_keystore()
        .delete(KEYCHAIN_SERVICE, name)
        .map_err(anyhow::Error::msg)?;
    ctx.emit(&SecretChangeReport {
        reference: r.to_string(),
        changed: existed,
    })?;
    Ok(exit::OK)
}

/// Hidden-input prompt; `None` when the user enters nothing.
fn prompt(label: &str) -> Result<Option<SecretString>> {
    let v = dialoguer::Password::new()
        .with_prompt(label)
        .allow_empty_password(true)
        .interact()
        .context("reading the value")?;
    Ok((!v.is_empty()).then(|| SecretString::from(v)))
}

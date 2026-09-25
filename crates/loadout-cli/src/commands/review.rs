//! `lo status`, `lo diff`, `lo approve`.

use std::fmt::Write as _;

use anyhow::{Result, bail};
use clap::Args;
use loadout_git::Git;
use loadout_model::config::normalize_url;
use schemars::JsonSchema;
use serde::Serialize;

use crate::commands::sync::exit_code;
use crate::ctx::{Ctx, Report};
use crate::engine::{self, Fetch, Options};
use crate::exit;
use crate::review::{PendingChange, Reason, State};
use crate::sources::{repo_dir, short};
use crate::state::Resolved;

/// Git's empty tree, to diff a new source against nothing.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Debug, Args)]
pub struct StatusArgs {}

/// `--json` output of `lo status`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusReport {
    /// RFC 3339 time of the last successful `lo sync`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<String>,
    pub sources: usize,
    pub enabled_items: usize,
    pub pending: Vec<PendingSummary>,
    /// True when a pending change is blocked by a critical audit finding.
    pub audit_blocked: bool,
    /// Audit findings (warn or worse) on pending changes.
    pub audit_findings: usize,
    /// True when some `kind/name` has an unresolved equal-rank conflict.
    pub conflicts: bool,
    /// Configuration fingerprint; equal on two machines iff
    /// their agent configuration is identical.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PendingSummary {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub to: String,
    pub reason: Reason,
}

impl Report for StatusReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        writeln!(
            out,
            "Last sync:  {}",
            self.last_sync.as_deref().unwrap_or("never")
        )?;
        writeln!(
            out,
            "Installed:  {} enabled item(s) from {} source(s)",
            self.enabled_items, self.sources
        )?;
        if self.pending.is_empty() {
            writeln!(out, "Pending:    none")?;
        } else {
            writeln!(out, "Pending:    {} change(s)", self.pending.len())?;
            for p in &self.pending {
                writeln!(
                    out,
                    "  {:<22} {} → {}  ({})",
                    p.source,
                    p.from.as_deref().map_or("(new)", short),
                    short(&p.to),
                    p.reason.describe()
                )?;
            }
        }
        if self.audit_findings > 0 {
            writeln!(
                out,
                "Audit:      {} finding(s) on pending changes{}",
                self.audit_findings,
                if self.audit_blocked { " (blocked)" } else { "" }
            )?;
        }
        if self.conflicts {
            writeln!(out, "Conflicts:  yes (see `lo why`)")?;
        }
        if let Some(fp) = &self.fingerprint {
            writeln!(out, "Fingerprint: {fp}")?;
        }
        Ok(())
    }
}

/// Exit code: 3 audit-blocked, 2 pending, 4 conflicts, else 0.
pub fn status(ctx: &Ctx, _args: StatusArgs) -> Result<u8> {
    let state = State::load(&ctx.paths.state_file())?;
    let resolved = Resolved::load(&ctx.paths.resolved_file())?;
    let report = StatusReport {
        last_sync: state.last_sync.clone(),
        sources: resolved.as_ref().map_or(0, |r| r.sources.len()),
        enabled_items: resolved
            .as_ref()
            .map_or(0, |r| r.items.iter().filter(|i| i.enabled).count()),
        audit_blocked: state.pending.iter().any(|p| p.reason == Reason::Blocked),
        audit_findings: state
            .pending
            .iter()
            .flat_map(|p| &p.findings)
            .filter(|f| f.severity >= loadout_audit::Severity::Warn)
            .count(),
        conflicts: resolved
            .as_ref()
            .is_some_and(|r| r.resolution.has_conflicts()),
        fingerprint: resolved.as_ref().map(crate::commands::share::fingerprint),
        pending: state
            .pending
            .iter()
            .map(|p| PendingSummary {
                source: p.source.clone(),
                from: p.from.clone(),
                to: p.to.clone(),
                reason: p.reason,
            })
            .collect(),
    };
    ctx.emit(&report)?;
    Ok(if report.audit_blocked {
        exit::AUDIT_BLOCKED
    } else if !report.pending.is_empty() {
        exit::PENDING
    } else if report.conflicts {
        exit::CONFLICTS
    } else {
        exit::OK
    })
}

#[derive(Debug, Args)]
pub struct DiffArgs {
    /// Only this source (its `LOADOUT.md` name).
    pub source: Option<String>,
}

/// `--json` output of `lo diff`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DiffReport {
    pub changes: Vec<DiffEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DiffEntry {
    #[serde(flatten)]
    pub change: PendingChange,
    /// `git diff` of the source repo between the applied and pending
    /// commits.
    pub diff: String,
}

impl Report for DiffReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if self.changes.is_empty() {
            return writeln!(out, "No pending changes.");
        }
        for d in &self.changes {
            let c = &d.change;
            writeln!(
                out,
                "== {} ({}): {} → {}  [{}]",
                c.source,
                c.url,
                c.from.as_deref().map_or("(new)", short),
                short(&c.to),
                c.reason.describe()
            )?;
            for (label, keys) in [
                ("added", &c.items.added),
                ("removed", &c.items.removed),
                ("changed", &c.items.changed),
            ] {
                if !keys.is_empty() {
                    writeln!(out, "   {label}: {}", keys.join(", "))?;
                }
            }
            for f in &c.findings {
                writeln!(
                    out,
                    "   audit {}: {} {}:{} {} [{}]",
                    f.severity, f.item, f.file, f.line, f.message, f.rule
                )?;
            }
            writeln!(out)?;
            out.push_str(&d.diff);
            if !d.diff.ends_with('\n') && !d.diff.is_empty() {
                out.push('\n');
            }
        }
        Ok(())
    }
}

fn select<'a>(state: &'a State, source: Option<&str>) -> Result<Vec<&'a PendingChange>> {
    let picked: Vec<&PendingChange> = state
        .pending
        .iter()
        .filter(|p| {
            source.is_none_or(|s| p.source == s || normalize_url(&p.url) == normalize_url(s))
        })
        .collect();
    if let Some(s) = source
        && picked.is_empty()
    {
        bail!("no pending change for {s:?} (see `lo status`)");
    }
    Ok(picked)
}

pub fn diff(ctx: &Ctx, args: DiffArgs) -> Result<u8> {
    let state = State::load(&ctx.paths.state_file())?;
    let git = Git::new();
    let mut changes = Vec::new();
    for p in select(&state, args.source.as_deref())? {
        let dir = repo_dir(&ctx.paths, &p.url);
        let from = p.from.as_deref().unwrap_or(EMPTY_TREE);
        let diff = git
            .diff(&dir, from, &p.to)
            .unwrap_or_else(|e| format!("(diff unavailable: {e})\n"));
        changes.push(DiffEntry {
            change: p.clone(),
            diff,
        });
    }
    let pending = !changes.is_empty();
    ctx.emit(&DiffReport { changes })?;
    Ok(if pending { exit::PENDING } else { exit::OK })
}

#[derive(Debug, Args)]
pub struct ApproveArgs {
    /// Approve this source's pending change (its `LOADOUT.md` name).
    #[arg(required_unless_present = "all", conflicts_with = "all")]
    pub source: Option<String>,
    /// Approve every pending change.
    #[arg(long)]
    pub all: bool,
    /// Also approve changes blocked by critical audit findings.
    #[arg(long)]
    pub force_audit: bool,
}

pub fn approve(ctx: &Ctx, args: ApproveArgs) -> Result<u8> {
    let state = State::load(&ctx.paths.state_file())?;
    let picked = select(&state, args.source.as_deref())?;
    if picked.is_empty() {
        bail!("nothing is pending");
    }
    let blocked: Vec<&str> = picked
        .iter()
        .filter(|p| p.reason == Reason::Blocked)
        .map(|p| p.source.as_str())
        .collect();
    if !blocked.is_empty() && !args.force_audit {
        if ctx.json {
            println!(
                "{}",
                serde_json::json!({ "error": format!("blocked by critical audit findings: {}; review with `lo diff`, then approve with --force-audit", blocked.join(", ")) })
            );
        } else {
            eprintln!(
                "error: blocked by critical audit findings: {}\n  review with `lo diff`, then approve with --force-audit",
                blocked.join(", ")
            );
        }
        return Ok(exit::AUDIT_BLOCKED);
    }
    let approved = picked
        .iter()
        .map(|p| (normalize_url(&p.url), p.to.clone()))
        .collect();
    let config = ctx.load_config()?;
    let report = engine::apply(
        ctx,
        &config,
        Options {
            approved,
            ..Options::new(Fetch::Cached)
        },
    )?;
    ctx.emit(&report)?;
    Ok(exit_code(&report))
}

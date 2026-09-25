//! `lo sync` — fetch sources, resolve, rebuild the store, reconcile
//! targets, and pin the result in `loadout.lock`.

use std::fmt::Write as _;

use anyhow::Result;
use clap::Args;

use crate::ctx::{Ctx, Report};
use crate::engine::{self, ApplyReport, Fetch, Options, Refresh};
use crate::exit;
use crate::review::State;

#[derive(Debug, Args)]
pub struct SyncArgs {
    /// Don't fetch: install exactly what `loadout.lock` pins (CI,
    /// reproducibility). Fails if the lock doesn't cover every item.
    #[arg(long, conflicts_with_all = ["yes", "dry_run"])]
    pub locked: bool,
    /// Plan only: fetch and show what would change, change nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Also apply changes that would wait for review (audit-blocked ones
    /// still need --force-audit).
    #[arg(long)]
    pub yes: bool,
    /// Apply changes even when the audit finds critical issues.
    #[arg(long)]
    pub force_audit: bool,
    /// Do nothing if the last successful sync is newer than `sync_interval`.
    #[arg(long)]
    pub if_stale: bool,
    /// Never prompt (sync doesn't prompt; accepted for scripts and CI).
    #[arg(long)]
    pub non_interactive: bool,
}

impl Report for ApplyReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if self.up_to_date {
            return writeln!(out, "Up to date (synced within sync_interval).");
        }
        if self.dry_run {
            writeln!(out, "Dry run: nothing was changed.")?;
        }
        if self.sources.is_empty() {
            writeln!(
                out,
                "No sources subscribed. Add one with `lo subscribe <url>`."
            )?;
        }
        for s in &self.sources {
            let moved = s
                .updated_from
                .as_deref()
                .map(|f| format!("  (was {})", &f[..f.len().min(8)]))
                .unwrap_or_default();
            let via = s
                .via
                .as_deref()
                .map(|v| format!("  (upstream of {v})"))
                .unwrap_or_default();
            writeln!(
                out,
                "{:<24} {}  {} item{}{}{moved}{via}",
                s.name,
                &s.commit[..s.commit.len().min(8)],
                s.items,
                if s.items == 1 { "" } else { "s" },
                if s.cached { "  (cached)" } else { "" }
            )?;
        }
        for t in &self.targets {
            writeln!(
                out,
                "{}: {} added, {} updated, {} removed, {} unchanged",
                t.id,
                t.added.len(),
                t.updated.len(),
                t.removed.len(),
                t.unchanged
            )?;
            for e in &t.errors {
                writeln!(out, "  error: {}: {}", e.path, e.message)?;
            }
        }
        for e in &self.signature_failures {
            writeln!(out, "SIGNATURE CHECK FAILED: {e}")?;
        }
        for f in &self.findings {
            writeln!(
                out,
                "audit {}: {} {}:{} {} [{}]",
                f.severity, f.item, f.file, f.line, f.message, f.rule
            )?;
        }
        if !self.pending.is_empty() {
            writeln!(
                out,
                "{} change(s) held for review: `lo diff` to inspect, `lo approve` to apply.",
                self.pending.len()
            )?;
        }
        if let Some(p) = &self.project {
            writeln!(out, "Project layer:")?;
            p.human(out)?;
        }
        Ok(())
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// Exit code for `sync`: errors, then blocked by audit or signature (3), then pending
/// review (2), then conflicts (4).
pub fn sync_exit_code(report: &ApplyReport) -> u8 {
    if report.has_errors() {
        exit::ERROR
    } else if report.audit_blocked || !report.signature_failures.is_empty() {
        exit::AUDIT_BLOCKED
    } else if !report.pending.is_empty() {
        exit::PENDING
    } else {
        exit_code(report)
    }
}

/// Exit code for an apply: errors beat conflicts.
pub fn exit_code(report: &ApplyReport) -> u8 {
    if report.has_errors() {
        exit::ERROR
    } else if report.conflicts {
        exit::CONFLICTS
    } else {
        exit::OK
    }
}

pub fn run(ctx: &Ctx, args: SyncArgs) -> Result<u8> {
    let config = ctx.load_config()?;
    let now = engine::unix_now();
    if args.if_stale && !stale(ctx, &config, now)? {
        ctx.emit(&ApplyReport {
            up_to_date: true,
            ..ApplyReport::default()
        })?;
        return Ok(exit::OK);
    }
    let opts = if args.locked {
        Options::new(Fetch::Locked)
    } else {
        Options {
            refresh: if args.dry_run {
                Refresh::Never
            } else {
                Refresh::IfDue
            },
            approve_all: args.yes,
            force_audit: args.force_audit,
            dry_run: args.dry_run,
            ..Options::new(Fetch::Remote)
        }
    };
    let mut report = engine::apply(ctx, &config, Options { now, ..opts })?;
    // Inside a repo with a project layer, sync that too.
    if ctx.paths.project.is_none()
        && let Some(root) = std::env::current_dir()
            .ok()
            .and_then(|d| crate::paths::Paths::find_project(&d))
    {
        let pctx = ctx.project(&root);
        let pconfig = pctx.load_config()?;
        let popts = if args.locked {
            Options::new(Fetch::Locked)
        } else {
            Options {
                approve_all: args.yes,
                force_audit: args.force_audit,
                dry_run: args.dry_run,
                ..Options::new(Fetch::Remote)
            }
        };
        report.project = Some(Box::new(engine::apply(
            &pctx,
            &pconfig,
            Options { now, ..popts },
        )?));
    }
    if !ctx.json
        && let Some(p) = &report.project
    {
        let extra: Vec<String> = p.warnings.iter().map(|w| format!("project: {w}")).collect();
        report.warnings.extend(extra);
    }
    ctx.emit(&report)?;
    let code = sync_exit_code(&report);
    Ok(match &report.project {
        Some(p) if code == exit::OK => sync_exit_code(p),
        _ => code,
    })
}

/// Whether the last successful sync is older than `sync_interval`.
fn stale(ctx: &Ctx, config: &loadout_model::Config, now: i64) -> Result<bool> {
    let state = State::load(&ctx.paths.state_file())?;
    let Some(last) = state.last_sync.as_deref() else {
        return Ok(true);
    };
    let last = loadout_model::time::parse_rfc3339(last)?;
    let interval = loadout_model::duration::parse_duration(config.sync_interval())?;
    Ok(now - last >= i64::try_from(interval.as_secs()).unwrap_or(i64::MAX))
}

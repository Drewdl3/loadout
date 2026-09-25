//! `lo audit [<source>|--all]` — run the content audit over
//! the items of your sources at their applied commits.

use std::fmt::Write as _;

use anyhow::{Result, bail};
use clap::Args;
use loadout_audit::{Finding, Severity};
use loadout_git::Git;
use loadout_model::config::normalize_url;
use schemars::JsonSchema;
use serde::Serialize;

use crate::ctx::{Ctx, Report};
use crate::exit;
use crate::membership;
use crate::scan::scan_source;
use crate::state::Resolved;

#[derive(Debug, Args)]
pub struct AuditArgs {
    /// Only this source (its `LOADOUT.md` name). Default: all sources.
    #[arg(conflicts_with = "all")]
    pub source: Option<String>,
    /// All sources (the default).
    #[arg(long)]
    pub all: bool,
    /// Audit the source repo checked out at this directory instead (e.g. in
    /// the repo's own CI).
    #[arg(long, conflicts_with_all = ["source", "all"])]
    pub path: Option<std::path::PathBuf>,
}

/// `--json` output of `lo audit`.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct AuditReport {
    /// Sources audited.
    pub sources: Vec<String>,
    /// Number of items audited.
    pub items: usize,
    pub findings: Vec<Finding>,
    /// The worst severity found, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worst: Option<Severity>,
    pub warnings: Vec<String>,
}

impl Report for AuditReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        for f in &self.findings {
            writeln!(
                out,
                "{:<8} {}  {}:{}  {} [{}]",
                f.severity, f.item, f.file, f.line, f.message, f.rule
            )?;
            if !f.excerpt.is_empty() {
                writeln!(out, "         > {}", f.excerpt)?;
            }
        }
        let n = self.findings.len();
        writeln!(
            out,
            "{} item(s) in {} source(s) audited: {}.",
            self.items,
            self.sources.len(),
            if n == 0 {
                "no findings".to_owned()
            } else {
                format!(
                    "{n} finding(s), worst {}",
                    self.worst.unwrap_or(Severity::Info)
                )
            }
        )
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// Exit code: 3 when a critical finding would block applying.
pub fn run(ctx: &Ctx, args: AuditArgs) -> Result<u8> {
    if let Some(dir) = &args.path {
        let scanned = scan_source(dir)?;
        let rules = crate::audit::rules(ctx, None)?;
        let mut report = AuditReport {
            sources: vec![scanned.manifest.name.clone()],
            items: scanned.items.len(),
            warnings: scanned.warnings.clone(),
            ..AuditReport::default()
        };
        for item in &scanned.items {
            report.findings.extend(crate::audit::audit(&rules, item));
        }
        return finish(ctx, report);
    }
    let Some(resolved) = Resolved::load(&ctx.paths.resolved_file())? else {
        bail!("nothing synced yet; run `lo sync`");
    };
    let config = ctx.load_config()?;
    let company_config = match &config.company_config {
        Some(url) => Some(membership::load_company_config(
            ctx,
            &Git::new(),
            url,
            false,
        )?),
        None => None,
    };
    let rules = crate::audit::rules(ctx, company_config.as_ref())?;
    let mut report = AuditReport::default();
    if let Some(name) = &args.source
        && !resolved.sources.iter().any(|s| &s.name == name)
    {
        bail!("no source named {name:?} (see `lo list`)");
    }
    for src in &resolved.sources {
        if args.source.as_ref().is_some_and(|n| n != &src.name) {
            continue;
        }
        let dir = loadout_git::repo_dir(&ctx.paths.repos_dir(), &normalize_url(&src.url));
        match scan_source(&dir) {
            Ok(scanned) => {
                for item in &scanned.items {
                    report.findings.extend(crate::audit::audit(&rules, item));
                }
                report.items += scanned.items.len();
                report.sources.push(src.name.clone());
            }
            Err(e) => report.warnings.push(format!("{}: {e:#}", src.name)),
        }
    }
    finish(ctx, report)
}

fn finish(ctx: &Ctx, mut report: AuditReport) -> Result<u8> {
    report.worst = loadout_audit::worst(&report.findings);
    ctx.emit(&report)?;
    Ok(if report.worst == Some(Severity::Critical) {
        exit::AUDIT_BLOCKED
    } else {
        exit::OK
    })
}

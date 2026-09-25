//! `lo targets [list|enable <id>|disable <id>|mode <id> symlink|copy]`:
//! which AI tools Loadout installs into, and how.

use std::fmt::Write as _;

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use loadout_model::LinkMode;
use loadout_targets::{TargetDef, TargetTable, expand_path};
use schemars::JsonSchema;
use serde::Serialize;

use crate::commands::sync::exit_code;
use crate::ctx::{Ctx, Report};
use crate::engine::{self, ApplyReport, Fetch, Options, display_path, enabled_targets};
use crate::exit;

#[derive(Debug, Args)]
pub struct TargetsArgs {
    #[command(subcommand)]
    pub command: Option<TargetsCommand>,
}

#[derive(Debug, Subcommand)]
pub enum TargetsCommand {
    /// Known targets, whether they're detected and enabled (the default).
    List,
    /// Install into this target from now on.
    Enable { id: String },
    /// Stop installing into this target (removes what Loadout put there).
    Disable { id: String },
    /// How to place items in this target: `symlink` (junctions on Windows)
    /// or `copy`.
    Mode {
        id: String,
        #[arg(value_parser = ["symlink", "copy"])]
        mode: String,
    },
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TargetInfo {
    pub id: String,
    pub display: String,
    /// The tool's directory exists on this machine.
    pub detected: bool,
    pub enabled: bool,
    #[schemars(with = "String")]
    pub mode: LinkMode,
    /// Where each kind of item goes (`~`-relative).
    pub paths: Vec<KindPath>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct KindPath {
    /// `skills`, `agents`, `extras.<type>`, `mcp`.
    pub kind: String,
    pub path: String,
}

/// `--json` output of `lo targets`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TargetsReport {
    pub targets: Vec<TargetInfo>,
    /// True when `[targets] enabled` is unset and detected tools are used.
    pub auto_detect: bool,
    /// The re-apply after a change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<ApplyReport>,
}

impl Report for TargetsReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        for t in &self.targets {
            writeln!(
                out,
                "{} {:<12} {:<12} {}{}",
                if t.enabled { "on " } else { "off" },
                t.id,
                t.display,
                t.mode.as_str(),
                if t.detected { "" } else { "  (not detected)" }
            )?;
        }
        if self.auto_detect {
            writeln!(
                out,
                "Using detected tools; `lo targets enable|disable` fixes the list."
            )?;
        }
        if let Some(s) = &self.sync {
            s.human(out)?;
        }
        Ok(())
    }

    fn warnings(&self) -> &[String] {
        self.sync.as_ref().map_or(&[], |s| s.warnings.as_slice())
    }
}

fn info(ctx: &Ctx, t: &TargetDef, enabled: bool, mode: LinkMode) -> TargetInfo {
    let shown = |p: &str| {
        expand_path(p, &ctx.paths.home)
            .map_or_else(|| p.to_owned(), |x| display_path(&x, &ctx.paths.home))
    };
    let mut paths = Vec::new();
    if let Some(s) = &t.skills {
        paths.push(KindPath {
            kind: "skills".into(),
            path: shown(&s.path),
        });
    }
    if let Some(a) = &t.agents {
        paths.push(KindPath {
            kind: "agents".into(),
            path: shown(&a.path),
        });
    }
    for (ty, e) in &t.extras {
        paths.push(KindPath {
            kind: format!("extras.{ty}"),
            path: shown(&e.path),
        });
    }
    if let Some(m) = &t.mcp
        && let Some(p) = &m.path
    {
        paths.push(KindPath {
            kind: "mcp".into(),
            path: shown(p),
        });
    }
    TargetInfo {
        id: t.id.clone(),
        display: t.display.clone(),
        detected: t
            .detect
            .iter()
            .filter_map(|d| expand_path(d, &ctx.paths.home))
            .any(|p| p.exists()),
        enabled,
        mode,
        paths,
    }
}

pub fn run(ctx: &Ctx, args: TargetsArgs) -> Result<u8> {
    let table = TargetTable::load(&ctx.paths.user_targets_dir())?;
    let mut doc = ctx.load_config_doc()?;
    let config = doc.config();
    let current: Vec<String> = enabled_targets(&table, &config, &ctx.paths.home)
        .0
        .iter()
        .map(|t| t.id.clone())
        .collect();
    let known = |id: &str| -> Result<()> {
        if table.get(id).is_none() {
            let ids: Vec<&str> = table.iter().map(|t| t.id.as_str()).collect();
            bail!("unknown target {id:?} (known: {})", ids.join(", "));
        }
        Ok(())
    };
    let changed = match args.command.unwrap_or(TargetsCommand::List) {
        TargetsCommand::List => false,
        TargetsCommand::Enable { id } => {
            known(&id)?;
            let mut ids = current.clone();
            if !ids.contains(&id) {
                ids.push(id);
            }
            doc.set_targets_enabled(&ids);
            true
        }
        TargetsCommand::Disable { id } => {
            known(&id)?;
            let ids: Vec<String> = current.iter().filter(|t| **t != id).cloned().collect();
            doc.set_targets_enabled(&ids);
            true
        }
        TargetsCommand::Mode { id, mode } => {
            known(&id)?;
            let mode = if mode == "copy" {
                LinkMode::Copy
            } else {
                LinkMode::Symlink
            };
            doc.set_target_mode(&id, mode);
            true
        }
    };
    let mut sync = None;
    if changed {
        ctx.save_config(&doc)?;
        let config = doc.config();
        // Apply only once something has been synced (nothing to re-apply before).
        if ctx.paths.resolved_file().exists() {
            sync = Some(engine::apply(ctx, &config, Options::new(Fetch::Cached))?);
        }
    }
    let config = doc.config();
    let enabled: Vec<String> = enabled_targets(&table, &config, &ctx.paths.home)
        .0
        .iter()
        .map(|t| t.id.clone())
        .collect();
    let report = TargetsReport {
        targets: table
            .iter()
            .map(|t| {
                info(
                    ctx,
                    t,
                    enabled.contains(&t.id),
                    config.targets.link_mode_for(&t.id),
                )
            })
            .collect(),
        auto_detect: config.targets.enabled.is_none(),
        sync,
    };
    ctx.emit(&report)?;
    Ok(report.sync.as_ref().map_or(exit::OK, exit_code))
}

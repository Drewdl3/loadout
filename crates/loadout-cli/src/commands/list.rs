//! `lo list` — resolved items from the last sync.

use std::fmt::Write as _;

use anyhow::{Result, bail};
use clap::Args;
use loadout_model::{ItemId, ItemKind};
use loadout_targets::TargetTable;
use schemars::JsonSchema;
use serde::Serialize;

use crate::ctx::{Ctx, Report};
use crate::exit;
use crate::state::{Resolved, ResolvedItem};

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Only items of this kind (skill, mcp, agent, plugin, extra).
    #[arg(long)]
    pub kind: Option<ItemKind>,
    /// Only enabled items.
    #[arg(long, conflicts_with = "disabled")]
    pub enabled: bool,
    /// Only disabled items.
    #[arg(long)]
    pub disabled: bool,
    /// Only items installed into this target.
    #[arg(long)]
    pub target: Option<String>,
}

/// `--json` output of `lo list`. With `--target`, items are that
/// target's winners, which can differ from the overall list when items
/// restrict their `targets`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ListReport {
    /// True once `lo sync` has run at least once.
    pub synced: bool,
    pub items: Vec<ResolvedItem>,
}

impl Report for ListReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if !self.synced {
            return writeln!(out, "Nothing synced yet. Run `lo sync`.");
        }
        if self.items.is_empty() {
            return writeln!(out, "No items.");
        }
        let width = self
            .items
            .iter()
            .map(|i| i.id.to_string().len())
            .max()
            .unwrap_or(0);
        for i in &self.items {
            let mark = if i.enabled { "on " } else { "off" };
            let id = i.id.to_string();
            match &i.description {
                Some(d) => writeln!(out, "{mark}  {id:<width$}  {}", one_line(d))?,
                None => writeln!(out, "{mark}  {id}")?,
            }
        }
        Ok(())
    }
}

fn one_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() > 80 {
        format!("{}…", line.chars().take(79).collect::<String>())
    } else {
        line.to_owned()
    }
}

pub fn run(ctx: &Ctx, args: ListArgs) -> Result<u8> {
    let Some(resolved) = Resolved::load(&ctx.paths.resolved_file())? else {
        ctx.emit(&ListReport {
            synced: false,
            items: Vec::new(),
        })?;
        return Ok(exit::OK);
    };
    let in_target: Option<Vec<ItemId>> = match &args.target {
        Some(id) => {
            let table = TargetTable::load(&ctx.paths.user_targets_dir())?;
            if table.get(id).is_none() {
                bail!("unknown target {id:?}");
            }
            Some(
                resolved
                    .placements
                    .get(id)
                    .map(|ps| ps.iter().map(|p| p.id.clone()).collect())
                    .unwrap_or_default(),
            )
        }
        None => None,
    };
    let items = resolved
        .items
        .into_iter()
        .filter(|i| args.kind.is_none_or(|k| i.kind == k))
        .filter(|i| !args.enabled || i.enabled)
        .filter(|i| !args.disabled || !i.enabled)
        .filter(|i| in_target.as_ref().is_none_or(|ids| ids.contains(&i.id)))
        .collect();
    ctx.emit(&ListReport {
        synced: true,
        items,
    })?;
    Ok(exit::OK)
}

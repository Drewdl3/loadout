//! `lo layers` — the layers in effect, with each rank and where it came
//! from; `lo layers set` / `unset` change your own ranks in `config.toml`.

use std::fmt::Write as _;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use loadout_model::RankOrigin;
use schemars::JsonSchema;
use serde::Serialize;

use crate::commands::sync::exit_code;
use crate::ctx::{Ctx, Report};
use crate::engine::{self, ApplyReport, Fetch, Options};
use crate::state::Resolved;
use crate::{exit, membership};

#[derive(Debug, Args)]
pub struct LayersArgs {
    #[command(subcommand)]
    pub command: Option<LayersCommand>,
}

#[derive(Debug, Subcommand)]
pub enum LayersCommand {
    /// Rank layers yourself, e.g. `lo layers set beta=27` (higher wins).
    Set {
        /// `name=rank` pairs.
        #[arg(
            required = true,
            value_name = "NAME=RANK",
            allow_negative_numbers = true
        )]
        ranks: Vec<String>,
    },
    /// Drop your own rank for layers, going back to their declared rank.
    Unset {
        #[arg(required = true, value_name = "NAME")]
        names: Vec<String>,
    },
}

/// `--json` output of `lo layers`, `lo layers set` and `lo layers unset`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LayersReport {
    /// Broadest (lowest rank) first.
    pub layers: Vec<LayerEntry>,
    /// Whether `config.toml` changed (always false for a listing).
    pub changed: bool,
    /// The re-apply after a change, from the cached checkouts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<ApplyReport>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LayerEntry {
    pub name: String,
    pub rank: i64,
    /// Where `rank` came from.
    pub from: RankOrigin,
    /// The rank its sources or the company config declared, when your
    /// `config.toml` changed it. Locks still hold at this rank.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared_rank: Option<i64>,
}

impl Report for LayersReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        writeln!(out, "Layers, broadest first (higher rank wins):")?;
        let width = self.layers.iter().map(|l| l.name.len()).max().unwrap_or(0);
        for l in &self.layers {
            let declared = l
                .declared_rank
                .map(|d| format!(" (declared {d}; locks hold there)"))
                .unwrap_or_default();
            writeln!(
                out,
                "  {:<width$}  {:>5}  from {}{declared}",
                l.name, l.rank, l.from
            )?;
        }
        match &self.sync {
            Some(sync) => sync.human(out),
            None if self.changed => writeln!(out, "Saved. Run `lo sync` to apply."),
            None => Ok(()),
        }
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

pub fn run(ctx: &Ctx, args: LayersArgs) -> Result<u8> {
    let changed = match args.command {
        None => false,
        Some(LayersCommand::Set { ranks }) => {
            let pairs = ranks
                .iter()
                .map(|p| parse_pair(p))
                .collect::<Result<Vec<_>>>()?;
            check_policy(ctx, pairs.iter().map(|(n, _)| n.as_str()))?;
            let mut doc = ctx.load_config_doc()?;
            let before = doc.to_string();
            for (name, rank) in &pairs {
                doc.set_layer(name, *rank);
            }
            let changed = doc.to_string() != before;
            if changed {
                ctx.save_config(&doc)?;
            }
            changed
        }
        Some(LayersCommand::Unset { names }) => {
            let mut doc = ctx.load_config_doc()?;
            let mut changed = false;
            for n in &names {
                changed |= doc.unset_layer(n.trim());
            }
            if changed {
                ctx.save_config(&doc)?;
            }
            changed
        }
    };
    // Re-apply right away when there is something synced to apply to.
    let sync = if changed && Resolved::load(&ctx.paths.resolved_file())?.is_some() {
        let config = ctx.load_config()?;
        Some(engine::apply(ctx, &config, Options::new(Fetch::Cached))?)
    } else {
        None
    };
    let (model, warnings) = crate::layers::current(ctx)?;
    let mut layers: Vec<LayerEntry> = model
        .entries()
        .map(|(name, l)| LayerEntry {
            name: name.to_owned(),
            rank: l.rank,
            from: l.from.clone(),
            declared_rank: l.declared.filter(|d| *d != l.rank),
        })
        .collect();
    layers.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.name.cmp(&b.name)));
    let code = sync.as_ref().map_or(exit::OK, exit_code);
    ctx.emit(&LayersReport {
        layers,
        changed,
        sync,
        warnings,
    })?;
    Ok(code)
}

/// `name=rank`.
fn parse_pair(p: &str) -> Result<(String, i64)> {
    let Some((name, rank)) = p.split_once('=') else {
        bail!("expected NAME=RANK, got {p:?}");
    };
    let name = name.trim();
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        bail!("layer names can't be empty or contain spaces, got {name:?}");
    }
    let rank = rank
        .trim()
        .parse()
        .with_context(|| format!("rank of {name} must be a whole number, got {rank:?}"))?;
    Ok((name.to_owned(), rank))
}

/// Refuses to re-rank the company config's layers when its policy forbids
/// it (`policy.allow_local_ranks: false`).
fn check_policy<'a>(ctx: &Ctx, names: impl Iterator<Item = &'a str>) -> Result<()> {
    let config = ctx.load_config()?;
    let Some(c) = membership::current_company_config(ctx, &config) else {
        return Ok(());
    };
    let c = c.company_config;
    if c.policy.allow_local_ranks {
        return Ok(());
    }
    for n in names {
        if c.layers.iter().any(|l| l.name == n) {
            bail!(
                "the company config doesn't allow re-ranking its layers, and {n:?} is one of them"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pairs() {
        assert_eq!(parse_pair("beta=27").unwrap(), ("beta".into(), 27));
        assert_eq!(parse_pair(" team = -5 ").unwrap(), ("team".into(), -5));
        assert!(parse_pair("beta").is_err());
        assert!(parse_pair("=3").is_err());
        assert!(parse_pair("be ta=3").is_err());
        assert!(parse_pair("beta=high").is_err());
    }
}

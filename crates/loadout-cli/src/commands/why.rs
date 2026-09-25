//! `lo why <kind/name>` — the explanation trail for one item.

use std::fmt::Write as _;

use anyhow::{Result, bail};
use clap::Args;
use loadout_core::resolve::{EnabledBy, Outcome, Rule, Trail};
use loadout_model::{ItemId, ItemKey, LinkMode};
use schemars::JsonSchema;
use serde::Serialize;

use crate::ctx::{Ctx, Report};
use crate::engine::display_path;
use crate::exit;
use crate::state::Resolved;

#[derive(Debug, Args)]
pub struct WhyArgs {
    /// `kind/name` (or a full `source:kind/name` id).
    pub item: String,
}

/// `--json` output of `lo why`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct WhyReport {
    pub key: ItemKey,
    pub winner: Option<ItemId>,
    pub rule: Option<Rule>,
    pub enabled: bool,
    pub enabled_by: Option<EnabledBy>,
    /// Every candidate, winner first.
    pub candidates: Vec<Trail>,
    /// Where the item goes in each enabled target.
    pub targets: Vec<TargetPlacement>,
    /// Resolution warnings about this item.
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TargetPlacement {
    pub target: String,
    /// The winner for this target, if the item is installed there.
    pub id: Option<ItemId>,
    pub path: Option<String>,
    pub mode: Option<LinkMode>,
    pub installed: bool,
}

pub fn run(ctx: &Ctx, args: WhyArgs) -> Result<u8> {
    let key = parse_key(&args.item)?;
    let Some(resolved) = Resolved::load(&ctx.paths.resolved_file())? else {
        bail!("nothing synced yet; run `lo sync`");
    };
    let Some(item) = resolved.resolution.get(&key) else {
        bail!("no source provides {key}; see `lo list`");
    };
    let targets = resolved
        .placements
        .iter()
        .map(
            |(target, ps)| match ps.iter().find(|p| p.id.key() == &key) {
                Some(p) => TargetPlacement {
                    target: target.clone(),
                    id: Some(p.id.clone()),
                    path: Some(display_path(&p.dest, &ctx.paths.home)),
                    mode: Some(p.mode),
                    installed: p.installed,
                },
                None => TargetPlacement {
                    target: target.clone(),
                    id: None,
                    path: None,
                    mode: None,
                    installed: false,
                },
            },
        )
        .collect();
    let needle = key.to_string();
    let warnings = resolved
        .resolution
        .warnings
        .iter()
        .map(ToString::to_string)
        .filter(|w| w.contains(&needle))
        .collect();
    ctx.emit(&WhyReport {
        key,
        winner: item.winner.clone(),
        rule: item.rule,
        enabled: item.enabled,
        enabled_by: item.enabled_by,
        candidates: item.candidates.clone(),
        targets,
        warnings,
    })?;
    Ok(exit::OK)
}

/// Accepts `kind/name` or `source:kind/name`.
pub fn parse_key(s: &str) -> Result<ItemKey> {
    if s.contains(':') {
        return Ok(s.parse::<ItemId>()?.key().clone());
    }
    Ok(s.parse()?)
}

impl Report for WhyReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        let state = match (self.winner.is_some(), self.enabled, self.enabled_by) {
            (false, _, _) => "not available to you".to_owned(),
            (true, true, Some(EnabledBy::Required)) => "enabled (required)".to_owned(),
            (true, true, Some(EnabledBy::Toggle)) => "enabled (your toggle)".to_owned(),
            (true, true, _) => "enabled (default)".to_owned(),
            (true, false, Some(EnabledBy::Toggle)) => "disabled (your toggle)".to_owned(),
            (true, false, _) => "disabled (default-off; `lo enable` to turn on)".to_owned(),
        };
        writeln!(out, "{} — {state}", self.key)?;
        if let (Some(w), Some(rule)) = (&self.winner, self.rule) {
            writeln!(out, "winner: {w} ({})", rule_text(rule))?;
        }
        writeln!(out, "candidates:")?;
        let width = self
            .candidates
            .iter()
            .map(|c| c.id.to_string().len())
            .max()
            .unwrap_or(0);
        for c in &self.candidates {
            let mark = if c.outcome == Outcome::Winner {
                "✓"
            } else {
                "✗"
            };
            let at = match &c.group {
                Some(u) => format!("{}:{u}", c.layer),
                None => c.layer.clone(),
            };
            let rank = c.rank.map_or("?".into(), |r| r.to_string());
            let mut flags = Vec::new();
            if c.locked {
                flags.push("locked");
            }
            if !c.overridable {
                flags.push("not overridable");
            }
            let flags = if flags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", flags.join(", "))
            };
            writeln!(
                out,
                "  {mark} {:<width$}  {at} (rank {rank}, priority {}){flags} — {}",
                c.id.to_string(),
                c.priority,
                outcome_text(&c.outcome)
            )?;
        }
        if !self.targets.is_empty() {
            writeln!(out, "targets:")?;
            for t in &self.targets {
                match (&t.id, &t.path) {
                    (Some(id), Some(path)) => {
                        let how = match (t.installed, t.mode) {
                            (false, _) => "NOT installed: path is occupied (see `lo doctor`)",
                            (true, Some(LinkMode::Copy)) => "copy",
                            (true, _) => "link",
                        };
                        let via = if Some(id) == self.winner.as_ref() {
                            String::new()
                        } else {
                            format!(" from {id}")
                        };
                        writeln!(out, "  {}: {path}{via} ({how})", t.target)?;
                    }
                    _ => writeln!(out, "  {}: not installed", t.target)?,
                }
            }
        }
        Ok(())
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

fn rule_text(rule: Rule) -> &'static str {
    match rule {
        Rule::Only => "only candidate",
        Rule::Locked => "locked or not overridable at the most general layer",
        Rule::HighestRank => "most specific layer",
        Rule::Priority => "same layer rank, higher source priority",
        Rule::Preferred => "same layer rank, your `lo prefer` choice",
        Rule::ConflictTieBreak => {
            "CONFLICT: same layer rank and priority, first source id; use `lo prefer`"
        }
    }
}

fn outcome_text(o: &Outcome) -> String {
    match o {
        Outcome::Winner => "winner".into(),
        Outcome::NotMember => "you are not a member of this group".into(),
        Outcome::UnknownLayer => "unknown layer".into(),
        Outcome::AppliesTo { axis } => format!("applies_to does not match your {axis}"),
        Outcome::TargetExcluded => "excluded by its targets list".into(),
        Outcome::Outranked { by } => format!("outranked by {by}"),
        Outcome::BlockedOverride { by } => format!("blocked override: {by} is locked"),
        Outcome::LostToLocked { by } => format!("lost to locked {by}"),
        Outcome::LowerPriority { by } => format!("lower priority than {by}"),
        Outcome::NotPreferred { by } => format!("you prefer {by}"),
        Outcome::ConflictTieBreak { by } => format!("conflict; tie broken in favour of {by}"),
    }
}

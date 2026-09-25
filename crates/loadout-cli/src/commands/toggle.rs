//! `lo enable` / `lo disable` / `lo prefer`: record a choice in
//! `config.toml`, then re-apply from the cached checkouts (no fetch).

use anyhow::{Result, bail};
use clap::Args;
use loadout_core::resolve::{EnabledBy, Outcome};
use loadout_model::{ItemId, ItemKey};

use crate::commands::sync::exit_code;
use crate::commands::why::parse_key;
use crate::ctx::Ctx;
use crate::engine::{self, Fetch, Options};
use crate::state::Resolved;

#[derive(Debug, Args)]
pub struct ToggleArgs {
    /// `kind/name` (the current winner) or a full `source:kind/name` id.
    pub item: String,
}

#[derive(Debug, Args)]
pub struct PreferArgs {
    /// `source:kind/name` — the candidate that should win its conflict.
    pub item: String,
}

fn load_resolved(ctx: &Ctx) -> Result<Resolved> {
    match Resolved::load(&ctx.paths.resolved_file())? {
        Some(r) => Ok(r),
        None => bail!("nothing synced yet; run `lo sync`"),
    }
}

pub fn enable(ctx: &Ctx, args: ToggleArgs) -> Result<u8> {
    set(ctx, &args.item, true)
}

pub fn disable(ctx: &Ctx, args: ToggleArgs) -> Result<u8> {
    set(ctx, &args.item, false)
}

fn set(ctx: &Ctx, item: &str, on: bool) -> Result<u8> {
    let resolved = load_resolved(ctx)?;
    let key = parse_key(item)?;
    let Some(entry) = resolved.resolution.get(&key) else {
        bail!("no source provides {key}; see `lo list`");
    };
    let id: ItemId = if item.contains(':') {
        let id: ItemId = item.parse()?;
        if !entry.candidates.iter().any(|c| c.id == id) {
            bail!("{id} is not provided by any subscribed source");
        }
        id
    } else {
        match &entry.winner {
            Some(w) => w.clone(),
            None => bail!("{}", unavailable(&key, entry)),
        }
    };
    if !on && entry.winner.as_ref() == Some(&id) && entry.enabled_by == Some(EnabledBy::Required) {
        bail!("{id} cannot be disabled: it is required or locked by its layer");
    }
    if on && entry.winner.as_ref() != Some(&id) {
        let why = entry
            .candidates
            .iter()
            .find(|c| c.id == id)
            .map(|c| &c.outcome);
        if let Some(o) = why
            && o.is_filtered()
        {
            bail!("{}", unavailable(&key, entry));
        }
        tracing::warn!("{id} is not the winner for {key}; the toggle has no effect until it wins");
    }

    let mut doc = ctx.load_config_doc()?;
    doc.set_toggle(&id.to_string(), on);
    ctx.save_config(&doc)?;
    reapply(ctx)
}

fn unavailable(key: &ItemKey, entry: &loadout_core::resolve::Resolved) -> String {
    let reasons: Vec<String> = entry
        .candidates
        .iter()
        .map(|c| match &c.outcome {
            Outcome::NotMember => format!(
                "{}: not a member of {}",
                c.id,
                c.group.as_deref().unwrap_or(&c.layer)
            ),
            Outcome::AppliesTo { axis } => {
                format!("{}: applies_to does not match your {axis}", c.id)
            }
            Outcome::UnknownLayer => format!("{}: unknown layer {}", c.id, c.layer),
            other => format!("{}: {other:?}", c.id),
        })
        .collect();
    format!(
        "{key} is not available to you ({}); see `lo why {key}`",
        reasons.join("; ")
    )
}

pub fn prefer(ctx: &Ctx, args: PreferArgs) -> Result<u8> {
    let id: ItemId = args.item.parse()?;
    let resolved = load_resolved(ctx)?;
    let Some(entry) = resolved.resolution.get(id.key()) else {
        bail!("no source provides {}; see `lo list`", id.key());
    };
    let Some(trail) = entry.candidates.iter().find(|c| c.id == id) else {
        bail!("{id} is not provided by any subscribed source");
    };
    if trail.outcome.is_filtered() {
        bail!("{id} does not apply to you; see `lo why {}`", id.key());
    }
    let mut doc = ctx.load_config_doc()?;
    doc.set_prefer(&id.key().to_string(), id.source());
    ctx.save_config(&doc)?;
    reapply(ctx)
}

fn reapply(ctx: &Ctx) -> Result<u8> {
    let config = ctx.load_config()?;
    let report = engine::apply(ctx, &config, Options::new(Fetch::Cached))?;
    ctx.emit(&report)?;
    Ok(exit_code(&report))
}

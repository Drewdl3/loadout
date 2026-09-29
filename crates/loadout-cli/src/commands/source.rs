//! `lo source set` / `lo source unset` — where a source sits for you: its
//! label, layer, rank and priority, in its `[[source]]` entry.
//!
//! This works for any source you get: your own subscriptions, the ones
//! your company config lists (the entry only places those), and
//! upstreams (the entry subscribes you to the upstream directly, so it
//! stays if you drop the source that pulled it in).

use std::fmt::Write as _;

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use loadout_git::Git;
use loadout_model::config::normalize_url;
use loadout_model::{SourceEdit, SourceSub};
use schemars::JsonSchema;
use serde::Serialize;

use crate::commands::subscribe::checked_url;
use crate::commands::sync::exit_code;
use crate::ctx::{Ctx, Report};
use crate::engine::{self, ApplyReport, Fetch, Options};
use crate::state::Resolved;
use crate::{exit, membership};

#[derive(Debug, Args)]
pub struct SourceArgs {
    #[command(subcommand)]
    pub command: SourceCommand,
}

#[derive(Debug, Subcommand)]
pub enum SourceCommand {
    /// Set a source's label, layer, rank or priority.
    Set(SetArgs),
    /// Remove a source's label, layer, rank or priority.
    Unset(UnsetArgs),
}

#[derive(Debug, Args)]
pub struct SetArgs {
    /// The source: its URL, its name (from `LOADOUT.md`) or its label.
    pub source: String,
    /// Display name, shown instead of its name.
    #[arg(long)]
    pub label: Option<String>,
    /// Rank all its items at this layer, items that set their own
    /// `layer:` included.
    #[arg(long)]
    pub layer: Option<String>,
    /// Rank all its items at exactly this rank (higher wins).
    #[arg(long, allow_negative_numbers = true)]
    pub rank: Option<i64>,
    /// Tie-breaker among equal-rank sources; higher wins.
    #[arg(long, allow_negative_numbers = true)]
    pub priority: Option<i64>,
}

#[derive(Debug, Args)]
pub struct UnsetArgs {
    /// The source: its URL, its name (from `LOADOUT.md`) or its label.
    pub source: String,
    #[arg(long)]
    pub label: bool,
    #[arg(long)]
    pub layer: bool,
    #[arg(long)]
    pub rank: bool,
    #[arg(long)]
    pub priority: bool,
}

/// `--json` output of `lo source set` and `lo source unset`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SourceSetReport {
    pub url: String,
    /// Whether `config.toml` changed.
    pub changed: bool,
    /// The `[[source]]` entry now.
    pub source: SourceSub,
    /// The re-apply after a change, from the cached checkouts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<ApplyReport>,
}

impl Report for SourceSetReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if !self.changed {
            return writeln!(out, "Nothing changed for {}.", self.url);
        }
        let s = &self.source;
        let mut parts = Vec::new();
        if let Some(l) = &s.label {
            parts.push(format!("label {l:?}"));
        }
        if let Some(l) = &s.layer {
            parts.push(format!("layer {l}"));
        }
        if let Some(r) = s.rank {
            parts.push(format!("rank {r}"));
        }
        if s.priority != 0 {
            parts.push(format!("priority {}", s.priority));
        }
        let now = if parts.is_empty() {
            "its own layer and name".to_owned()
        } else {
            parts.join(", ")
        };
        writeln!(out, "{}: {now}.", self.url)?;
        match &self.sync {
            Some(sync) => sync.human(out),
            None => writeln!(out, "Run `lo sync` to apply."),
        }
    }

    fn warnings(&self) -> &[String] {
        self.sync.as_ref().map_or(&[], |s| &s.warnings)
    }
}

pub fn run(ctx: &Ctx, args: SourceArgs) -> Result<u8> {
    let (source, edit) = match args.command {
        SourceCommand::Set(a) => {
            let edit = SourceEdit {
                label: a
                    .label
                    .map(|l| Some(l.trim().to_owned()).filter(|l| !l.is_empty())),
                layer: a.layer.map(|l| Some(l.trim().to_owned())),
                rank: a.rank.map(Some),
                priority: a.priority.map(Some),
            };
            if edit.is_empty() {
                bail!("nothing to set: give --label, --layer, --rank or --priority");
            }
            (a.source, edit)
        }
        SourceCommand::Unset(a) => {
            let edit = SourceEdit {
                label: a.label.then_some(None),
                layer: a.layer.then_some(None),
                rank: a.rank.then_some(None),
                priority: a.priority.then_some(None),
            };
            if edit.is_empty() {
                bail!("nothing to unset: give --label, --layer, --rank or --priority");
            }
            (a.source, edit)
        }
    };
    let url = find_url(ctx, &source)?;
    check(ctx, &url, &edit)?;
    let mut doc = ctx.load_config_doc()?;
    let changed = doc.edit_source(&url, &edit);
    if changed {
        ctx.save_config(&doc)?;
    }
    let sync = if changed && Resolved::load(&ctx.paths.resolved_file())?.is_some() {
        let config = ctx.load_config()?;
        Some(engine::apply(ctx, &config, Options::new(Fetch::Cached))?)
    } else {
        None
    };
    let entry = doc
        .config()
        .source(&url)
        .cloned()
        .unwrap_or_else(|| SourceSub::new(url.clone()));
    let code = sync.as_ref().map_or(exit::OK, exit_code);
    ctx.emit(&SourceSetReport {
        url,
        changed,
        source: entry,
        sync,
    })?;
    Ok(code)
}

/// The URL of `source`: a synced source's name or label, a `[[source]]`
/// URL, or any other URL (which `set` then subscribes to).
fn find_url(ctx: &Ctx, source: &str) -> Result<String> {
    let source = source.trim();
    let synced = Resolved::load(&ctx.paths.resolved_file())?
        .map(|r| r.sources)
        .unwrap_or_default();
    if let Some(s) = synced
        .iter()
        .find(|s| s.name == source || s.label.as_deref() == Some(source))
    {
        if s.path.is_some() {
            bail!(
                "{source} is a nested source in {}; set the repo's root source instead",
                s.url
            );
        }
        return Ok(s.url.clone());
    }
    let config = ctx.load_config()?;
    if let Some(s) = config.source(source) {
        return Ok(s.url.clone());
    }
    let norm = normalize_url(source);
    if let Some(s) = synced.iter().find(|s| normalize_url(&s.url) == norm) {
        return Ok(s.url.clone());
    }
    if !source.contains(['/', '\\', ':']) {
        bail!("no source named {source:?}; give its URL, or see the Sources page of `lo ui`");
    }
    checked_url(source)
}

/// The layer must exist, and the company config may forbid re-ranking the
/// sources it lists, or subscribing to any others.
fn check(ctx: &Ctx, url: &str, edit: &SourceEdit) -> Result<()> {
    if let Some(Some(layer)) = &edit.layer {
        let (model, _) = crate::layers::current(ctx)?;
        if model.rank(layer).is_none() {
            bail!("no layer {layer:?}; define it first with `lo layers set {layer}=<rank>`");
        }
    }
    let config = ctx.load_config()?;
    let Some(company_url) = config
        .company_config
        .clone()
        .filter(|_| ctx.paths.project.is_none())
    else {
        return Ok(());
    };
    let Ok(c) = membership::load_company_config(ctx, &Git::new(), &company_url, false) else {
        return Ok(());
    };
    let listed =
        c.company_config.lists_source(url) || normalize_url(url) == normalize_url(&company_url);
    let reranks = matches!(edit.layer, Some(Some(_))) || matches!(edit.rank, Some(Some(_)));
    if listed && reranks && !c.company_config.policy.allow_local_ranks {
        bail!("the company config doesn't allow re-ranking the sources it lists ({url})");
    }
    let known = config.source(url).is_some()
        || listed
        || Resolved::load(&ctx.paths.resolved_file())?.is_some_and(|r| {
            r.sources
                .iter()
                .any(|s| normalize_url(&s.url) == normalize_url(url))
        });
    if !known && !c.company_config.policy.allow_manual_sources {
        bail!("your company config does not allow subscribing to sources it doesn't list ({url})");
    }
    Ok(())
}

//! `lo search <query> [--kind] [--tag] [--layer] [--all-sources]`.

use std::fmt::Write as _;

use anyhow::{Result, bail};
use clap::Args;
use loadout_git::Git;
use loadout_model::config::normalize_url;
use loadout_search::{Hit, Query, State};
use schemars::JsonSchema;
use serde::Serialize;

use crate::ctx::{Ctx, Report};
use crate::exit;
use crate::membership;
use crate::scan::{Overrides, scan_source_with};
use crate::sources::repo_dir;
use crate::state::Resolved;

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Words to look for in names, descriptions, tags and text (may be empty
    /// with filters).
    #[arg(default_value = "")]
    pub query: String,
    /// Only items of this kind (skill, mcp, agent, plugin, extra).
    #[arg(long)]
    pub kind: Option<String>,
    /// Only items with this tag.
    #[arg(long)]
    pub tag: Option<String>,
    /// Only items of this layer (team, org, role, …).
    #[arg(long)]
    pub layer: Option<String>,
    /// Only items for this role (`applies_to.role`, or no role limit).
    #[arg(long)]
    pub role: Option<String>,
    /// Only items of this group.
    #[arg(long)]
    pub group: Option<String>,
    /// Only items from this source (its `LOADOUT.md` name).
    #[arg(long)]
    pub source: Option<String>,
    /// Only items written or co-written by this person (part of the name).
    #[arg(long)]
    pub author: Option<String>,
    /// Also search every company config-listed source you can read, to find groups
    /// worth joining.
    #[arg(long)]
    pub all_sources: bool,
    /// At most this many results (0: all).
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
}

/// `--json` output of `lo search`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchReport {
    pub hits: Vec<Hit>,
    pub warnings: Vec<String>,
}

impl Report for SearchReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if self.hits.is_empty() {
            return writeln!(out, "No matches.");
        }
        let width = self.hits.iter().map(|h| h.doc.id.len()).max().unwrap_or(0);
        for h in &self.hits {
            let state = match h.doc.state {
                State::Enabled => "on  ",
                State::Disabled => "off ",
                State::Shadowed => "shad",
                State::NotForYou => "n/a ",
                State::NotSubscribed => "join",
            };
            let group = format!("{}:{}", h.doc.layer, h.doc.group.as_deref().unwrap_or("-"));
            let desc: String = h
                .doc
                .description
                .as_deref()
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(70)
                .collect();
            writeln!(out, "{state} {:<width$}  {group:<24} {desc}", h.doc.id)?;
        }
        if self
            .hits
            .iter()
            .any(|h| h.doc.state == State::NotSubscribed)
        {
            writeln!(
                out,
                "`join` items come from groups you're not in: `lo join <layer>:<group>` (if the group allows it)."
            )?;
        }
        Ok(())
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// Read-only checkouts of every company config-listed source you aren't subscribed
/// to (for discovery; nothing is installed), with the layer/group the
/// company config gives each. Unreadable ones become warnings.
pub fn unsubscribed_company_config_sources(
    ctx: &Ctx,
    warnings: &mut Vec<String>,
) -> Result<Vec<(std::path::PathBuf, Overrides)>> {
    let config = ctx.load_config()?;
    let Some(url) = &config.company_config else {
        bail!("--all-sources needs a company config (`lo init <company-config-url>`)");
    };
    let git = Git::new();
    let company_config = membership::load_company_config(ctx, &git, url, false)?;
    let subscribed: Vec<String> = Resolved::load(&ctx.paths.resolved_file())?
        .map(|r| r.sources.iter().map(|s| normalize_url(&s.url)).collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for group in &company_config.company_config.groups {
        for src in &group.sources {
            if subscribed.contains(&normalize_url(src)) {
                continue;
            }
            let dir = repo_dir(&ctx.paths, src);
            if let Err(e) = git.sync_checkout(src, &dir, None) {
                warnings.push(format!("{src}: can't read ({e})"));
                continue;
            }
            out.push((
                dir,
                Overrides {
                    layer: Some(group.layer.clone()),
                    group: Some(group.name.clone()),
                },
            ));
        }
    }
    Ok(out)
}

pub fn run(ctx: &Ctx, args: SearchArgs) -> Result<u8> {
    let Some(mut docs) = crate::search::load(&ctx.paths.search_file())? else {
        bail!("nothing synced yet; run `lo sync`");
    };
    let mut warnings = Vec::new();
    if args.all_sources {
        for (dir, overrides) in unsubscribed_company_config_sources(ctx, &mut warnings)? {
            match scan_source_with(&dir, &overrides) {
                Ok(scanned) => docs.extend(
                    scanned
                        .items
                        .iter()
                        .map(|i| crate::search::doc(i, State::NotSubscribed)),
                ),
                Err(e) => warnings.push(format!("{}: {e:#}", dir.display())),
            }
        }
    }
    let hits = loadout_search::search(
        &docs,
        &Query {
            text: args.query,
            kind: args.kind,
            tag: args.tag,
            layer: args.layer,
            role: args.role,
            group: args.group,
            source: args.source,
            author: args.author,
            limit: args.limit,
        },
    );
    ctx.emit(&SearchReport { hits, warnings })?;
    Ok(exit::OK)
}

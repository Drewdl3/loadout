//! `lo info <source|id>`: render a source's `LOADOUT.md` or an
//! item's documentation.

use std::fmt::Write as _;

use anyhow::{Context, Result, bail};
use clap::Args;
use loadout_model::config::normalize_url;
use loadout_model::frontmatter::Document;
use loadout_model::manifest::MANIFEST_FILE;
use loadout_model::{ItemId, ManifestDoc};
use schemars::JsonSchema;
use serde::Serialize;

use crate::commands::why::parse_key;
use crate::ctx::{Ctx, Report};
use crate::exit;
use crate::scan::main_file;
use crate::state::Resolved;
use crate::store;

#[derive(Debug, Args)]
pub struct InfoArgs {
    /// A source name (from `LOADOUT.md`), an item `kind/name`, or a full
    /// `source:kind/name` id.
    pub what: String,
}

/// `--json` output of `lo info`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum InfoReport {
    Source {
        name: String,
        url: String,
        commit: String,
        layer: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        group: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        owners: Vec<String>,
        /// Items of this source in the last sync (`kind/name`).
        items: Vec<String>,
        /// The Markdown body of `LOADOUT.md`.
        body: String,
    },
    Item {
        /// `source:kind/name`.
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        layer: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        group: Option<String>,
        enabled: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        author: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        co_authors: Vec<String>,
        /// The item's main file (`SKILL.md`, `<name>.md`), including
        /// frontmatter.
        text: String,
    },
}

impl Report for InfoReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        match self {
            InfoReport::Source {
                name,
                url,
                commit,
                layer,
                group,
                description,
                owners,
                items,
                body,
            } => {
                writeln!(
                    out,
                    "{name} — {}:{}",
                    layer,
                    group.as_deref().unwrap_or("-")
                )?;
                writeln!(out, "{url} @ {}", &commit[..commit.len().min(8)])?;
                if let Some(d) = description {
                    writeln!(out, "{d}")?;
                }
                if !owners.is_empty() {
                    writeln!(out, "Owners: {}", owners.join(", "))?;
                }
                writeln!(
                    out,
                    "Items: {}",
                    if items.is_empty() {
                        "none".into()
                    } else {
                        items.join(", ")
                    }
                )?;
                writeln!(out)?;
                out.push_str(body.trim_start());
                if !body.ends_with('\n') {
                    out.push('\n');
                }
                Ok(())
            }
            InfoReport::Item {
                id,
                layer,
                group,
                enabled,
                author,
                co_authors,
                text,
                ..
            } => {
                writeln!(
                    out,
                    "{id} — {}:{} — {}",
                    layer,
                    group.as_deref().unwrap_or("-"),
                    if *enabled { "enabled" } else { "disabled" }
                )?;
                if let Some(line) = byline(author.as_deref(), co_authors) {
                    writeln!(out, "{line}")?;
                }
                writeln!(out)?;
                out.push_str(text);
                if !text.ends_with('\n') {
                    out.push('\n');
                }
                Ok(())
            }
        }
    }
}

/// "By Ada, with Grace and Linus", or `None` when nobody is named.
fn byline(author: Option<&str>, co_authors: &[String]) -> Option<String> {
    let with = co_authors.join(", ");
    match (author, co_authors.is_empty()) {
        (None, true) => None,
        (Some(a), true) => Some(format!("By {a}")),
        (Some(a), false) => Some(format!("By {a}, with {with}")),
        (None, false) => Some(format!("With {with}")),
    }
}

pub fn run(ctx: &Ctx, args: InfoArgs) -> Result<u8> {
    let Some(resolved) = Resolved::load(&ctx.paths.resolved_file())? else {
        bail!("nothing synced yet; run `lo sync`");
    };
    let what = args.what.trim();
    if let Some(src) = resolved.sources.iter().find(|s| s.name == what) {
        let dir = loadout_git::repo_dir(&ctx.paths.repos_dir(), &normalize_url(&src.url));
        let text = std::fs::read_to_string(dir.join(MANIFEST_FILE))
            .with_context(|| format!("reading {MANIFEST_FILE} of {}", src.name))?;
        let doc = ManifestDoc::parse(&text)?;
        let body = Document::split(&text)?.body.to_owned();
        let m = doc.manifest;
        let items: Vec<String> = resolved
            .resolution
            .items
            .iter()
            .flat_map(|r| &r.candidates)
            .filter(|c| c.id.source() == src.name)
            .map(|c| c.id.key().to_string())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        ctx.emit(&InfoReport::Source {
            name: m.name,
            url: src.url.clone(),
            commit: src.commit.clone(),
            layer: m.layer,
            group: m.group,
            description: m.description,
            owners: m.owners,
            items,
            body,
        })?;
        return Ok(exit::OK);
    }
    let id: ItemId = if what.contains(':') {
        what.parse()?
    } else if what.contains('/') {
        let key = parse_key(what)?;
        match resolved.resolution.get(&key).and_then(|r| r.winner.clone()) {
            Some(w) => w,
            None => bail!("no source provides {key}; see `lo list`"),
        }
    } else {
        bail!(
            "{what:?} is neither a source name nor an item (use kind/name, e.g. skill/write-spec)"
        );
    };
    let Some(item) = resolved.items.iter().find(|i| i.id == id) else {
        bail!("{id} is not installed; see `lo why {}`", id.key());
    };
    let dir = ctx.paths.store_dir().join(store::item_rel_path(&id));
    let file = main_file(id.key());
    let text = std::fs::read_to_string(dir.join(&file))
        .with_context(|| format!("reading {file} of {id} from the store; run `lo sync`"))?;
    ctx.emit(&InfoReport::Item {
        id: id.to_string(),
        description: item.description.clone(),
        layer: item.layer.clone(),
        group: item.group.clone(),
        enabled: item.enabled,
        author: item.author.clone(),
        co_authors: item.co_authors.clone(),
        text,
    })?;
    Ok(exit::OK)
}

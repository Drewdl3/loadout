//! Building and caching the search index.

use std::path::Path;

use anyhow::{Context, Result};
use loadout_core::resolve::{Outcome, Resolution};
use loadout_model::frontmatter::Document;
use loadout_search::{Doc, State};
use loadout_targets::fsutil::atomic_write;

use crate::scan::{ScannedItem, main_file};

/// Longest body text indexed per item.
const MAX_BODY: usize = 20_000;

/// The searchable form of `item`.
pub fn doc(item: &ScannedItem, state: State) -> Doc {
    let key = item.id.key();
    let main = main_file(key);
    let body = item
        .files
        .iter()
        .find(|f| f.path == main)
        .and_then(|f| std::str::from_utf8(&f.contents).ok())
        .and_then(|t| Document::split(t).ok().map(|d| d.body.to_owned()))
        .unwrap_or_default();
    let body: String = body.chars().take(MAX_BODY).collect();
    Doc {
        id: item.id.to_string(),
        kind: key.kind().as_str().to_owned(),
        name: key.name().to_owned(),
        source: item.id.source().to_owned(),
        layer: item.loadout.layer.clone().unwrap_or_default(),
        group: item.loadout.group.clone(),
        tags: item.loadout.tags.clone(),
        product: item.loadout.product.clone(),
        roles: item
            .loadout
            .applies_to
            .get("role")
            .cloned()
            .unwrap_or_default(),
        description: item.meta.description.clone(),
        author: item.loadout.author.clone(),
        co_authors: item.loadout.co_authors.clone(),
        body,
        state,
    }
}

/// Where an item stands in `resolution`.
pub fn state_of(item: &ScannedItem, resolution: &Resolution) -> State {
    let Some(r) = resolution.get(item.id.key()) else {
        return State::NotForYou;
    };
    if r.winner.as_ref() == Some(&item.id) {
        return if r.enabled {
            State::Enabled
        } else {
            State::Disabled
        };
    }
    match r
        .candidates
        .iter()
        .find(|c| c.id == item.id)
        .map(|c| &c.outcome)
    {
        Some(
            Outcome::NotMember
            | Outcome::UnknownLayer
            | Outcome::AppliesTo { .. }
            | Outcome::TargetExcluded,
        )
        | None => State::NotForYou,
        Some(_) => State::Shadowed,
    }
}

pub fn save(path: &Path, docs: &[Doc]) -> Result<()> {
    let json = serde_json::to_vec(docs)?;
    atomic_write(path, &json, false).with_context(|| format!("writing {}", path.display()))
}

/// The cached index; `None` before the first sync.
pub fn load(path: &Path) -> Result<Option<Vec<Doc>>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(serde_json::from_slice(&b).with_context(|| {
            format!("{} is corrupt; run `lo sync`", path.display())
        })?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

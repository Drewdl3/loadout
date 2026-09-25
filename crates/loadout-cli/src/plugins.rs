//! Installing Claude Code plugins: a local marketplace
//! directory plus two keys in `~/.claude/settings.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use loadout_core::resolve::Resolution;
use loadout_model::{ItemId, ItemKind};
use loadout_targets::mcp::{Format, merge};
use loadout_targets::owned::{OwnedFile, OwnedPlugins};
use loadout_targets::plugins::{Bundle, settings_entries, write_marketplace};
use loadout_targets::{TargetDef, expand_path};

use crate::engine::{PathReport, TargetReport, display_path};
use crate::paths::Paths;
use crate::scan::ScannedItem;
use crate::store;

pub const RENDERER: &str = "claude-code-marketplace";
const MARKETPLACES_KEY: &str = "extraKnownMarketplaces";
const ENABLED_KEY: &str = "enabledPlugins";

/// Enabled plugin winners in `resolution`.
pub fn enabled_plugins(resolution: &Resolution) -> Vec<&ItemId> {
    resolution
        .items
        .iter()
        .filter(|r| r.enabled)
        .filter_map(|r| r.winner.as_ref())
        .filter(|id| id.key().kind() == ItemKind::Plugin)
        .collect()
}

/// Reconciles plugins for `target`. Returns false when the target can't
/// take plugins (the caller mentions it once).
#[allow(clippy::too_many_arguments)]
pub fn reconcile_target(
    paths: &Paths,
    target: &TargetDef,
    resolution: &Resolution,
    items: &BTreeMap<&ItemId, &ScannedItem>,
    owned: &mut OwnedFile,
    report: &mut TargetReport,
    warnings: &mut Vec<String>,
    dry_run: bool,
) -> bool {
    let Some(spec) = target.plugins.as_ref().filter(|s| s.renderer == RENDERER) else {
        return false;
    };
    if paths.project.is_some() {
        return false;
    }
    let Some(settings) = spec
        .path
        .as_deref()
        .and_then(|p| expand_path(p, &paths.home))
    else {
        warnings.push(format!("[plugins] of target {} needs a path", target.id));
        return true;
    };
    let ids = enabled_plugins(resolution);
    let prev = owned.plugins.get(&target.id).cloned().unwrap_or_default();
    if ids.is_empty() && prev.marketplaces.is_empty() && prev.enabled.is_empty() {
        return true;
    }
    let market = marketplace_dir(paths);
    let store = paths.store_dir();
    let dirs: Vec<PathBuf> = ids
        .iter()
        .map(|id| store.join(store::item_rel_path(id)))
        .collect();
    let versions: Vec<Option<String>> = ids
        .iter()
        .map(|id| {
            items[id]
                .meta
                .extra
                .get("version")
                .and_then(|v| v.as_str().map(str::to_owned))
        })
        .collect();
    let bundles: Vec<Bundle<'_>> = ids
        .iter()
        .zip(&dirs)
        .zip(&versions)
        .map(|((id, dir), version)| Bundle {
            name: id.key().name(),
            dir,
            description: items[id].meta.description.as_deref(),
            version: version.as_deref(),
        })
        .collect();
    if !dry_run && let Err(e) = write_marketplace(&market, &bundles) {
        report.errors.push(PathReport {
            item: "plugins".into(),
            path: display_path(&market, &paths.home),
            message: e.to_string(),
        });
        return true;
    }
    let names: Vec<&str> = bundles.iter().map(|b| b.name).collect();
    let (markets, enabled) = settings_entries(&market, &names);
    let display = display_path(&settings, &paths.home);
    let text = std::fs::read_to_string(&settings).unwrap_or_default();
    let result = merge(
        Format::ClaudeCodeJson,
        &display,
        &text,
        MARKETPLACES_KEY,
        &markets,
        &prev.marketplaces,
    )
    .and_then(|m1| {
        let text2 = m1.text.clone().unwrap_or(text.clone());
        merge(
            Format::ClaudeCodeJson,
            &display,
            &text2,
            ENABLED_KEY,
            &enabled,
            &prev.enabled,
        )
        .map(|m2| (m1, m2))
    });
    let (m1, m2) = match result {
        Ok(r) => r,
        Err(e) => {
            report.errors.push(PathReport {
                item: "plugins".into(),
                path: display,
                message: e.to_string(),
            });
            return true;
        }
    };
    let new_text = m2.text.clone().or(m1.text.clone());
    if let Some(new) = &new_text
        && !dry_run
        && let Err(e) = loadout_targets::fsutil::atomic_write(&settings, new.as_bytes(), true)
    {
        report.errors.push(PathReport {
            item: "plugins".into(),
            path: display,
            message: e.to_string(),
        });
        return true;
    }
    let key = |k: &String| {
        format!(
            "plugin/{}",
            k.trim_end_matches(&format!("@{}", loadout_targets::plugins::MARKETPLACE))
        )
    };
    report.added.extend(m2.added.iter().map(key));
    report.updated.extend(m2.updated.iter().map(key));
    report.removed.extend(m2.removed.iter().map(key));
    report.unchanged += m2.unchanged;
    for c in m1.collisions.iter().chain(&m2.collisions) {
        warnings.push(format!(
            "{}: {display} already has {c:?} not managed by Loadout; plugins not registered",
            target.id
        ));
    }
    let entry = OwnedPlugins {
        settings,
        marketplaces: m1.owned,
        enabled: m2.owned,
    };
    if entry.marketplaces.is_empty() && entry.enabled.is_empty() {
        owned.plugins.remove(&target.id);
    } else {
        owned.plugins.insert(target.id.clone(), entry);
    }
    true
}

/// Unregisters everything Loadout registered for `target_id`.
pub fn remove_all(
    home: &Path,
    target_id: &str,
    owned: &mut OwnedFile,
    report: &mut TargetReport,
    warnings: &mut Vec<String>,
    dry_run: bool,
) {
    let Some(prev) = owned.plugins.get(target_id).cloned() else {
        return;
    };
    let display = display_path(&prev.settings, home);
    let text = std::fs::read_to_string(&prev.settings).unwrap_or_default();
    let empty = BTreeMap::new();
    let r = merge(
        Format::ClaudeCodeJson,
        &display,
        &text,
        MARKETPLACES_KEY,
        &empty,
        &prev.marketplaces,
    )
    .and_then(|m1| {
        let t = m1.text.clone().unwrap_or(text.clone());
        merge(
            Format::ClaudeCodeJson,
            &display,
            &t,
            ENABLED_KEY,
            &empty,
            &prev.enabled,
        )
        .map(|m2| (m1, m2))
    });
    match r {
        Ok((m1, m2)) => {
            if let Some(new) = m2.text.or(m1.text)
                && !dry_run
                && let Err(e) =
                    loadout_targets::fsutil::atomic_write(&prev.settings, new.as_bytes(), true)
            {
                warnings.push(format!("{target_id}: could not update {display}: {e}"));
                return;
            }
            report.removed.extend(
                m2.removed
                    .iter()
                    .map(|k| format!("plugin/{}", k.split('@').next().unwrap_or(k))),
            );
            owned.plugins.remove(target_id);
        }
        Err(e) => warnings.push(format!("{target_id}: could not update {display}: {e}")),
    }
}

/// `<data>/claude-marketplace`.
pub fn marketplace_dir(paths: &Paths) -> PathBuf {
    paths.data_dir.join("claude-marketplace")
}

/// Targets that can't take plugins, for the one-line note.
pub fn unsupported_note(
    targets: &BTreeSet<String>,
    plugins: usize,
    project: bool,
) -> Option<String> {
    if plugins == 0 || targets.is_empty() {
        return None;
    }
    let names: Vec<&str> = targets.iter().map(String::as_str).collect();
    Some(if project {
        format!(
            "{plugins} plugin(s) not installed: plugins are installed for your user only, not in project mode"
        )
    } else {
        format!(
            "{plugins} plugin(s) installed only into Claude Code; {} don't support plugins",
            names.join(", ")
        )
    })
}

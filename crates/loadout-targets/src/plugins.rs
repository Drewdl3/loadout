//! Claude Code plugins: Loadout writes a local marketplace
//! directory holding the enabled plugin bundles and registers it in Claude
//! Code's settings. Pure helpers plus the directory writer.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde_json::{Value as Json, json};

/// The marketplace name Loadout registers (`<plugin>@loadout-sync`).
pub const MARKETPLACE: &str = "loadout-sync";

/// One plugin to publish in the marketplace.
#[derive(Debug, Clone)]
pub struct Bundle<'a> {
    pub name: &'a str,
    /// The plugin's files in the store.
    pub dir: &'a Path,
    pub description: Option<&'a str>,
    pub version: Option<&'a str>,
}

/// `.claude-plugin/marketplace.json` for `bundles`.
pub fn marketplace_json(bundles: &[Bundle<'_>]) -> Json {
    let plugins: Vec<Json> = bundles
        .iter()
        .map(|b| {
            let mut e = json!({ "name": b.name, "source": format!("./{}", b.name) });
            if let Some(d) = b.description {
                e["description"] = json!(d);
            }
            if let Some(v) = b.version {
                e["version"] = json!(v);
            }
            e
        })
        .collect();
    json!({
        "name": MARKETPLACE,
        "owner": { "name": "Loadout" },
        "description": "Plugins installed by Loadout (`lo sync`). Managed automatically.",
        "plugins": plugins,
    })
}

/// A plugin's `.claude-plugin/plugin.json`, generated from `PLUGIN.md`
/// when the bundle doesn't ship one.
pub fn plugin_json(b: &Bundle<'_>) -> Json {
    let mut m = json!({ "name": b.name });
    if let Some(d) = b.description {
        m["description"] = json!(d);
    }
    if let Some(v) = b.version {
        m["version"] = json!(v);
    }
    m
}

/// The settings entries registering the marketplace at `dir` and enabling
/// `names`: (`extraKnownMarketplaces` entries, `enabledPlugins` entries).
pub fn settings_entries(
    dir: &Path,
    names: &[&str],
) -> (BTreeMap<String, Json>, BTreeMap<String, Json>) {
    let mut markets = BTreeMap::new();
    if !names.is_empty() {
        markets.insert(
            MARKETPLACE.to_owned(),
            json!({ "source": { "source": "directory", "path": dir.to_string_lossy() } }),
        );
    }
    let enabled = names
        .iter()
        .map(|n| (format!("{n}@{MARKETPLACE}"), json!(true)))
        .collect();
    (markets, enabled)
}

/// Rewrites the marketplace directory at `dir` to hold exactly `bundles`
/// (staged next to it, then swapped in).
pub fn write_marketplace(dir: &Path, bundles: &[Bundle<'_>]) -> io::Result<()> {
    let staging = dir.with_extension("new");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(staging.join(".claude-plugin"))?;
    std::fs::write(
        staging.join(".claude-plugin/marketplace.json"),
        serde_json::to_vec_pretty(&marketplace_json(bundles)).map_err(io::Error::other)?,
    )?;
    for b in bundles {
        let to = staging.join(b.name);
        copy_tree(b.dir, &to)?;
        let manifest = to.join(".claude-plugin/plugin.json");
        if !manifest.exists() {
            std::fs::create_dir_all(manifest.parent().expect("parent"))?;
            std::fs::write(
                &manifest,
                serde_json::to_vec_pretty(&plugin_json(b)).map_err(io::Error::other)?,
            )?;
        }
    }
    let old = dir.with_extension("old");
    if old.exists() {
        std::fs::remove_dir_all(&old)?;
    }
    if dir.exists() {
        std::fs::rename(dir, &old)?;
    }
    std::fs::rename(&staging, dir)?;
    if old.exists() {
        std::fs::remove_dir_all(&old)?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &dest)?;
        } else {
            std::fs::copy(e.path(), &dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_marketplace_with_generated_manifests() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store/acme/plugin/review-kit");
        std::fs::create_dir_all(store.join("commands")).unwrap();
        std::fs::write(store.join("PLUGIN.md"), "---\nname: review-kit\n---\n").unwrap();
        std::fs::write(store.join("commands/review.md"), "Review.").unwrap();
        let market = tmp.path().join("claude-marketplace");
        let b = Bundle {
            name: "review-kit",
            dir: &store,
            description: Some("Review helpers."),
            version: Some("1.2.0"),
        };
        write_marketplace(&market, std::slice::from_ref(&b)).unwrap();
        let m: Json = serde_json::from_slice(
            &std::fs::read(market.join(".claude-plugin/marketplace.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(m["name"], MARKETPLACE);
        assert_eq!(m["owner"]["name"], "Loadout");
        assert_eq!(m["plugins"][0]["source"], "./review-kit");
        let p: Json = serde_json::from_slice(
            &std::fs::read(market.join("review-kit/.claude-plugin/plugin.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            p,
            json!({"name": "review-kit", "description": "Review helpers.", "version": "1.2.0"})
        );
        assert!(market.join("review-kit/commands/review.md").is_file());

        // Rewriting drops plugins that are gone.
        write_marketplace(&market, &[]).unwrap();
        assert!(!market.join("review-kit").exists());
        assert!(!tmp.path().join("claude-marketplace.new").exists());
    }

    #[test]
    fn settings_entries_shape() {
        let (m, e) = settings_entries(Path::new("/d/market"), &["a", "b"]);
        assert_eq!(m[MARKETPLACE]["source"]["source"], "directory");
        assert_eq!(m[MARKETPLACE]["source"]["path"], "/d/market");
        assert_eq!(
            e.keys().collect::<Vec<_>>(),
            ["a@loadout-sync", "b@loadout-sync"]
        );
        let (m, e) = settings_entries(Path::new("/d"), &[]);
        assert!(m.is_empty() && e.is_empty());
    }
}

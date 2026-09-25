//! The store: `data/store/<source>/<kind>/<name>/`, the materialized
//! winning items from which targets are linked. Keyed by source
//! as well as name because different targets may resolve different
//! winners for the same `kind/name`.
//!
//! The store is rebuilt from scratch on every apply: a new tree is written
//! to `store.new/` and swapped in, so a crash leaves either the old or the
//! new store in place, never a mix.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use loadout_model::ItemId;

use crate::scan::ItemFile;

/// Rebuilds the store at `store` from `items`.
pub fn rebuild<'a, I>(store: &Path, items: I) -> Result<()>
where
    I: IntoIterator<Item = (&'a ItemId, &'a [ItemFile])>,
{
    recover(store)?;
    let staging = sibling(store, "new");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging).with_context(|| format!("creating {}", staging.display()))?;
    for (id, files) in items {
        let item_dir = staging.join(item_rel_path(id));
        for f in files {
            let dest = item_dir.join(&f.path);
            std::fs::create_dir_all(dest.parent().unwrap())?;
            std::fs::write(&dest, &f.contents)
                .with_context(|| format!("writing {}", dest.display()))?;
        }
    }
    swap_in(store, &staging)
}

/// The store-relative directory of an item: `<source>/<kind>/<name>`.
pub fn item_rel_path(id: &ItemId) -> PathBuf {
    Path::new(id.source())
        .join(id.key().kind().as_str())
        .join(id.key().name())
}

/// Finishes or rolls back an interrupted swap.
pub fn recover(store: &Path) -> Result<()> {
    let old = sibling(store, "old");
    if old.exists() {
        if store.exists() {
            std::fs::remove_dir_all(&old)?;
        } else {
            std::fs::rename(&old, store)?;
        }
    }
    Ok(())
}

fn swap_in(store: &Path, staging: &Path) -> Result<()> {
    let old = sibling(store, "old");
    if store.exists() {
        std::fs::rename(store, &old)
            .with_context(|| format!("moving aside {}", store.display()))?;
    }
    std::fs::rename(staging, store).with_context(|| format!("installing {}", store.display()))?;
    if old.exists() {
        std::fs::remove_dir_all(&old)?;
    }
    Ok(())
}

fn sibling(store: &Path, suffix: &str) -> PathBuf {
    let mut name = store.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(suffix);
    store.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(pairs: &[(&str, &str)]) -> Vec<ItemFile> {
        pairs
            .iter()
            .map(|(p, c)| ItemFile {
                path: p.to_string(),
                contents: c.as_bytes().to_vec(),
            })
            .collect()
    }

    #[test]
    fn rebuild_replaces_contents() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let a: ItemId = "src:skill/a".parse().unwrap();
        let b: ItemId = "src:skill/b".parse().unwrap();
        let fa = files(&[("SKILL.md", "a"), ("ref/x.md", "x")]);
        let fb = files(&[("SKILL.md", "b")]);

        rebuild(&store, [(&a, fa.as_slice()), (&b, fb.as_slice())]).unwrap();
        assert_eq!(
            std::fs::read_to_string(store.join("src/skill/a/ref/x.md")).unwrap(),
            "x"
        );
        assert!(store.join("src/skill/b/SKILL.md").is_file());

        rebuild(&store, [(&b, fb.as_slice())]).unwrap();
        assert!(!store.join("src/skill/a").exists());
        assert!(store.join("src/skill/b/SKILL.md").is_file());
        assert!(!tmp.path().join("store.new").exists());
        assert!(!tmp.path().join("store.old").exists());
    }

    #[test]
    fn recovers_from_interrupted_swap() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let old = tmp.path().join("store.old");
        std::fs::create_dir_all(old.join("skill/a")).unwrap();
        // Crash after moving the old store aside but before installing the new one.
        recover(&store).unwrap();
        assert!(store.join("skill/a").is_dir());
        assert!(!old.exists());

        // Crash after installing the new store but before deleting the old one.
        std::fs::create_dir_all(&old).unwrap();
        recover(&store).unwrap();
        assert!(store.join("skill/a").is_dir());
        assert!(!old.exists());
    }
}

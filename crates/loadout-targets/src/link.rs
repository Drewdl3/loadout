//! Placing store entries into target directories: symlinks (Unix), NTFS
//! junctions (Windows, no admin rights needed) or copies.

use std::io;
use std::path::{Path, PathBuf};

use loadout_core::links::{Action, Existing, Owned, Plan};
use loadout_model::LinkMode;

/// Reports what is on disk at `path` without following links.
pub fn inspect(path: &Path) -> Existing {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Existing::Missing;
    };
    let ft = meta.file_type();
    if ft.is_symlink() || is_junction(path) {
        match std::fs::read_link(path) {
            Ok(target) => Existing::Link(normalize_link_target(target)),
            Err(_) => Existing::Link(PathBuf::new()),
        }
    } else if ft.is_dir() {
        Existing::Dir
    } else {
        Existing::File
    }
}

#[cfg(windows)]
fn is_junction(path: &Path) -> bool {
    junction::exists(path).unwrap_or(false)
}

#[cfg(not(windows))]
fn is_junction(_path: &Path) -> bool {
    false
}

/// Strips the Windows verbatim prefix (`\\?\`, `\??\`) that junction
/// targets carry, so they compare equal to the paths we created them with.
fn normalize_link_target(target: PathBuf) -> PathBuf {
    let s = target.to_string_lossy();
    for prefix in [r"\\?\", r"\??\"] {
        if let Some(rest) = s.strip_prefix(prefix)
            && !rest.starts_with("UNC\\")
        {
            return PathBuf::from(rest);
        }
    }
    target
}

/// Places `src` (a store directory or file) at `dest`.
pub fn place(src: &Path, dest: &Path, mode: LinkMode) -> io::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match mode {
        LinkMode::Symlink => symlink(src, dest),
        LinkMode::Copy => {
            // Copy next to the destination, then rename: a crash never
            // leaves a half-copied entry at `dest`.
            let tmp = temp_sibling(dest);
            if inspect(&tmp) != Existing::Missing {
                remove(&tmp)?;
            }
            copy_recursive(src, &tmp)?;
            std::fs::rename(&tmp, dest).inspect_err(|_| {
                let _ = remove(&tmp);
            })
        }
    }
}

/// `<dest>.loadout-tmp`, where copies are staged.
fn temp_sibling(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".loadout-tmp");
    dest.with_file_name(name)
}

#[cfg(unix)]
fn symlink(src: &Path, dest: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(src, dest)
}

#[cfg(windows)]
fn symlink(src: &Path, dest: &Path) -> io::Result<()> {
    if src.is_dir() {
        junction::create(src, dest)
    } else {
        std::os::windows::fs::symlink_file(src, dest).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "{e}; file symlinks on Windows need Developer Mode — \
                     use `link_mode = \"copy\"` for this target"
                ),
            )
        })
    }
}

fn copy_recursive(src: &Path, dest: &Path) -> io::Result<()> {
    if src.is_file() {
        std::fs::copy(src, dest)?;
        return Ok(());
    }
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_recursive(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Removes an entry Loadout owns. Links are removed without touching
/// what they point to.
pub fn remove(path: &Path) -> io::Result<()> {
    match inspect(path) {
        Existing::Missing => Ok(()),
        Existing::Link(_) => remove_link(path),
        Existing::Dir => std::fs::remove_dir_all(path),
        Existing::File => std::fs::remove_file(path),
    }
}

#[cfg(unix)]
fn remove_link(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path)
}

#[cfg(windows)]
fn remove_link(path: &Path) -> io::Result<()> {
    // Directory symlinks and junctions are removed with RemoveDirectory,
    // which deletes the link itself, never the target's contents.
    std::fs::remove_dir(path).or_else(|_| std::fs::remove_file(path))
}

/// Ownership to record *before* executing `plan` (crash safety): the
/// current records, plus every entry the plan will create, marked with a
/// content hash that never matches so an interrupted placement is redone.
/// Entries being replaced or removed keep their current record until the
/// final ownership is saved.
pub fn intent(before: &[Owned], plan: &Plan) -> Vec<Owned> {
    let mut out: Vec<Owned> = before.to_vec();
    for action in &plan.actions {
        if let Action::Create(d) = action
            && !out.iter().any(|o| o.dest == d.dest)
        {
            out.push(Owned {
                item: d.item.clone(),
                dest: d.dest.clone(),
                mode: d.mode,
                content_hash: PENDING_HASH.into(),
            });
        }
    }
    out.sort_by(|a, b| a.dest.cmp(&b.dest));
    out
}

/// Content hash recorded for an entry whose placement hasn't completed.
pub const PENDING_HASH: &str = "blake3:pending";

/// Outcome of applying a [`Plan`].
#[derive(Debug, Default)]
pub struct Applied {
    /// Ownership records to persist: the plan's records minus entries whose
    /// placement failed.
    pub owned: Vec<Owned>,
    /// Failures, by destination.
    pub errors: Vec<(PathBuf, io::Error)>,
}

/// Executes the filesystem side of `plan`.
pub fn apply(plan: &Plan) -> Applied {
    let mut errors = Vec::new();
    let mut failed = std::collections::BTreeSet::new();
    for action in &plan.actions {
        let result = match action {
            Action::Create(d) => place(&d.src, &d.dest, d.mode),
            Action::Replace(d) => remove(&d.dest).and_then(|()| place(&d.src, &d.dest, d.mode)),
            Action::Remove(o) => remove(&o.dest),
            Action::Keep(_) | Action::Collision { .. } | Action::Disown { .. } => Ok(()),
        };
        if let Err(e) = result {
            failed.insert(action.dest().to_path_buf());
            errors.push((action.dest().to_path_buf(), e));
        }
        if !matches!(
            action,
            Action::Keep(_) | Action::Collision { .. } | Action::Disown { .. }
        ) {
            crate::testhook::crash_point("link");
        }
    }
    let mut owned: Vec<Owned> = plan
        .owned
        .iter()
        .filter(|o| !failed.contains(&o.dest))
        .cloned()
        .collect();
    // A failed removal is still ours; keep tracking it so the next sync retries.
    for action in &plan.actions {
        if let Action::Remove(o) = action
            && failed.contains(&o.dest)
        {
            owned.push(o.clone());
        }
    }
    owned.sort_by(|a, b| a.dest.cmp(&b.dest));
    Applied { owned, errors }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loadout_core::links::{Desired, plan};

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store/skill/a");
        std::fs::create_dir_all(store.join("ref")).unwrap();
        std::fs::write(store.join("SKILL.md"), "a").unwrap();
        std::fs::write(store.join("ref/x.md"), "x").unwrap();
        let target = tmp.path().join("home/.claude/skills");
        (tmp, store, target)
    }

    fn desired(src: &Path, target: &Path, mode: LinkMode) -> Desired {
        Desired {
            item: "skill/a".parse().unwrap(),
            dest: target.join("a"),
            src: src.to_path_buf(),
            mode,
            content_hash: "h".into(),
        }
    }

    #[test]
    fn symlink_mode_links_and_is_idempotent() {
        let (_tmp, src, target) = setup();
        let d = desired(&src, &target, LinkMode::Symlink);
        let p = plan(std::slice::from_ref(&d), &[], inspect);
        let applied = apply(&p);
        assert!(applied.errors.is_empty(), "{:?}", applied.errors);
        assert_eq!(inspect(&d.dest), Existing::Link(src.clone()));
        assert_eq!(
            std::fs::read_to_string(d.dest.join("ref/x.md")).unwrap(),
            "x"
        );

        let again = plan(std::slice::from_ref(&d), &applied.owned, inspect);
        assert!(again.actions.iter().all(|a| matches!(a, Action::Keep(_))));
    }

    #[test]
    fn copy_mode_copies_tree() {
        let (_tmp, src, target) = setup();
        let d = desired(&src, &target, LinkMode::Copy);
        let applied = apply(&plan(std::slice::from_ref(&d), &[], inspect));
        assert!(applied.errors.is_empty());
        assert_eq!(inspect(&d.dest), Existing::Dir);
        assert_eq!(
            std::fs::read_to_string(d.dest.join("ref/x.md")).unwrap(),
            "x"
        );
    }

    #[test]
    fn removing_a_link_leaves_the_store_intact() {
        let (_tmp, src, target) = setup();
        let d = desired(&src, &target, LinkMode::Symlink);
        let applied = apply(&plan(std::slice::from_ref(&d), &[], inspect));
        let removed = apply(&plan(&[], &applied.owned, inspect));
        assert!(removed.errors.is_empty());
        assert!(removed.owned.is_empty());
        assert_eq!(inspect(&d.dest), Existing::Missing);
        assert!(src.join("SKILL.md").is_file());
    }

    #[test]
    fn unowned_directory_is_never_touched() {
        let (_tmp, src, target) = setup();
        let d = desired(&src, &target, LinkMode::Symlink);
        std::fs::create_dir_all(&d.dest).unwrap();
        std::fs::write(d.dest.join("mine.md"), "user data").unwrap();
        let p = plan(std::slice::from_ref(&d), &[], inspect);
        let applied = apply(&p);
        assert!(matches!(p.actions[0], Action::Collision { .. }));
        assert!(applied.owned.is_empty());
        assert_eq!(
            std::fs::read_to_string(d.dest.join("mine.md")).unwrap(),
            "user data"
        );
    }

    #[test]
    fn switching_modes_replaces_entry() {
        let (_tmp, src, target) = setup();
        let link = desired(&src, &target, LinkMode::Symlink);
        let owned = apply(&plan(std::slice::from_ref(&link), &[], inspect)).owned;
        let copy = desired(&src, &target, LinkMode::Copy);
        let applied = apply(&plan(std::slice::from_ref(&copy), &owned, inspect));
        assert!(applied.errors.is_empty(), "{:?}", applied.errors);
        assert_eq!(inspect(&copy.dest), Existing::Dir);
        assert!(src.join("SKILL.md").is_file(), "store must survive");
        let back = apply(&plan(std::slice::from_ref(&link), &applied.owned, inspect));
        assert!(back.errors.is_empty());
        assert_eq!(inspect(&link.dest), Existing::Link(src.clone()));
    }

    #[test]
    fn verbatim_prefixes_are_stripped() {
        assert_eq!(
            normalize_link_target(PathBuf::from(r"\\?\C:\x\y")),
            PathBuf::from(r"C:\x\y")
        );
        assert_eq!(
            normalize_link_target(PathBuf::from(r"\??\C:\x")),
            PathBuf::from(r"C:\x")
        );
        assert_eq!(
            normalize_link_target(PathBuf::from("/a/b")),
            PathBuf::from("/a/b")
        );
    }
}

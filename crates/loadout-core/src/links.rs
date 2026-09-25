//! Reconciling target directories with the store.
//!
//! [`plan`] decides, without touching the filesystem, which entries to
//! create, replace, keep or remove. Loadout only ever touches entries it
//! owns; anything else at a wanted path is reported as a collision and left
//! alone.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use loadout_model::{ItemKey, LinkMode};
use serde::{Deserialize, Serialize};

/// An entry Loadout wants in a target directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desired {
    pub item: ItemKey,
    /// Where the entry goes, e.g. `~/.claude/skills/write-spec`.
    pub dest: PathBuf,
    /// The store path it mirrors.
    pub src: PathBuf,
    pub mode: LinkMode,
    pub content_hash: String,
}

/// A target entry Loadout created (persisted in `owned.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owned {
    pub item: ItemKey,
    pub dest: PathBuf,
    pub mode: LinkMode,
    pub content_hash: String,
}

/// What is currently on disk at a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Existing {
    Missing,
    /// A symlink or junction, with its target.
    Link(PathBuf),
    Dir,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing at `dest`: place the entry.
    Create(Desired),
    /// An owned entry is stale: remove it, then place the entry.
    Replace(Desired),
    /// Already correct.
    Keep(Desired),
    /// An owned entry is no longer wanted: remove it.
    Remove(Owned),
    /// Something Loadout does not own occupies `dest`: skip and warn.
    Collision { wanted: Desired, found: Existing },
    /// An owned entry was replaced by the user; forget it, leave it alone.
    Disown { owned: Owned, found: Existing },
}

impl Action {
    pub fn dest(&self) -> &Path {
        match self {
            Action::Create(d) | Action::Replace(d) | Action::Keep(d) => &d.dest,
            Action::Collision { wanted, .. } => &wanted.dest,
            Action::Remove(o) | Action::Disown { owned: o, .. } => &o.dest,
        }
    }
}

/// The result of planning: actions in `dest` order, plus the ownership
/// records that will hold once they are applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub actions: Vec<Action>,
    pub owned: Vec<Owned>,
}

/// Plans how to make the target entries match `desired`.
///
/// `existing` reports what is on disk at a path. Paths are compared as
/// given; callers pass them in one canonical form.
pub fn plan(desired: &[Desired], owned: &[Owned], existing: impl Fn(&Path) -> Existing) -> Plan {
    let owned_by_dest: BTreeMap<&Path, &Owned> =
        owned.iter().map(|o| (o.dest.as_path(), o)).collect();
    let mut wanted_dests = BTreeSet::new();
    let mut actions = Vec::new();

    for d in desired {
        wanted_dests.insert(d.dest.as_path());
        let found = existing(&d.dest);
        let action = match (owned_by_dest.get(d.dest.as_path()), found) {
            (_, Existing::Missing) => Action::Create(d.clone()),
            (Some(o), found) => {
                if !matches_owned_shape(o.mode, &found) {
                    // The user replaced our entry with their own.
                    actions.push(Action::Disown {
                        owned: (*o).clone(),
                        found: found.clone(),
                    });
                    Action::Collision {
                        wanted: d.clone(),
                        found,
                    }
                } else if is_current(d, o, &found) {
                    Action::Keep(d.clone())
                } else {
                    Action::Replace(d.clone())
                }
            }
            (None, Existing::Link(target)) if d.mode == LinkMode::Symlink && target == d.src => {
                // Our link, but ownership was lost (e.g. owned.json deleted): adopt it.
                Action::Keep(d.clone())
            }
            (None, found) => Action::Collision {
                wanted: d.clone(),
                found,
            },
        };
        actions.push(action);
    }

    for o in owned {
        if wanted_dests.contains(o.dest.as_path()) {
            continue;
        }
        match existing(&o.dest) {
            Existing::Missing => {}
            found if matches_owned_shape(o.mode, &found) => actions.push(Action::Remove(o.clone())),
            found => actions.push(Action::Disown {
                owned: o.clone(),
                found,
            }),
        }
    }

    actions.sort_by(|a, b| a.dest().cmp(b.dest()).then(rank(a).cmp(&rank(b))));

    let mut new_owned: Vec<Owned> = actions
        .iter()
        .filter_map(|a| match a {
            Action::Create(d) | Action::Replace(d) | Action::Keep(d) => Some(Owned {
                item: d.item.clone(),
                dest: d.dest.clone(),
                mode: d.mode,
                content_hash: d.content_hash.clone(),
            }),
            _ => None,
        })
        .collect();
    new_owned.sort_by(|a, b| a.dest.cmp(&b.dest));

    Plan {
        actions,
        owned: new_owned,
    }
}

fn rank(a: &Action) -> u8 {
    match a {
        Action::Disown { .. } => 0,
        _ => 1,
    }
}

/// Whether `found` still looks like what Loadout created in `mode`.
fn matches_owned_shape(mode: LinkMode, found: &Existing) -> bool {
    match mode {
        LinkMode::Symlink => matches!(found, Existing::Link(_)),
        LinkMode::Copy => matches!(found, Existing::Dir | Existing::File),
    }
}

fn is_current(d: &Desired, o: &Owned, found: &Existing) -> bool {
    match (d.mode, o.mode, found) {
        (LinkMode::Symlink, LinkMode::Symlink, Existing::Link(t)) => *t == d.src,
        (LinkMode::Copy, LinkMode::Copy, Existing::Dir | Existing::File) => {
            o.content_hash == d.content_hash
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn desired(name: &str, mode: LinkMode, hash: &str) -> Desired {
        Desired {
            item: format!("skill/{name}").parse().unwrap(),
            dest: PathBuf::from(format!("/t/{name}")),
            src: PathBuf::from(format!("/store/skill/{name}")),
            mode,
            content_hash: hash.into(),
        }
    }

    fn owned_from(d: &Desired) -> Owned {
        Owned {
            item: d.item.clone(),
            dest: d.dest.clone(),
            mode: d.mode,
            content_hash: d.content_hash.clone(),
        }
    }

    fn fs(entries: &[(&str, Existing)]) -> impl Fn(&Path) -> Existing + use<> {
        let map: HashMap<PathBuf, Existing> = entries
            .iter()
            .map(|(p, e)| (PathBuf::from(p), e.clone()))
            .collect();
        move |p| map.get(p).cloned().unwrap_or(Existing::Missing)
    }

    fn link(to: &str) -> Existing {
        Existing::Link(PathBuf::from(to))
    }

    #[test]
    fn creates_missing_entries_and_owns_them() {
        let d = desired("a", LinkMode::Symlink, "h1");
        let p = plan(std::slice::from_ref(&d), &[], fs(&[]));
        assert_eq!(p.actions, [Action::Create(d.clone())]);
        assert_eq!(p.owned, [owned_from(&d)]);
    }

    #[test]
    fn keeps_correct_symlink_even_if_content_changed() {
        let d = desired("a", LinkMode::Symlink, "h2");
        let mut o = owned_from(&d);
        o.content_hash = "h1".into();
        let p = plan(
            std::slice::from_ref(&d),
            &[o],
            fs(&[("/t/a", link("/store/skill/a"))]),
        );
        assert_eq!(p.actions, [Action::Keep(d.clone())]);
        assert_eq!(p.owned[0].content_hash, "h2");
    }

    #[test]
    fn replaces_owned_link_pointing_elsewhere() {
        let d = desired("a", LinkMode::Symlink, "h");
        let p = plan(
            std::slice::from_ref(&d),
            &[owned_from(&d)],
            fs(&[("/t/a", link("/old/store/skill/a"))]),
        );
        assert_eq!(p.actions, [Action::Replace(d)]);
    }

    #[test]
    fn copy_mode_replaces_only_when_content_changes() {
        let d = desired("a", LinkMode::Copy, "h2");
        let mut o = owned_from(&d);
        let on_disk = fs(&[("/t/a", Existing::Dir)]);
        assert_eq!(
            plan(std::slice::from_ref(&d), std::slice::from_ref(&o), &on_disk).actions,
            [Action::Keep(d.clone())]
        );
        o.content_hash = "h1".into();
        assert_eq!(
            plan(std::slice::from_ref(&d), &[o], &on_disk).actions,
            [Action::Replace(d)]
        );
    }

    #[test]
    fn switching_link_mode_replaces() {
        let d = desired("a", LinkMode::Copy, "h");
        let o = owned_from(&desired("a", LinkMode::Symlink, "h"));
        let p = plan(
            std::slice::from_ref(&d),
            &[o],
            fs(&[("/t/a", link("/store/skill/a"))]),
        );
        assert_eq!(p.actions, [Action::Replace(d)]);
    }

    #[test]
    fn unowned_entry_is_a_collision_and_untouched() {
        let d = desired("a", LinkMode::Symlink, "h");
        for found in [Existing::Dir, Existing::File, link("/elsewhere")] {
            let p = plan(
                std::slice::from_ref(&d),
                &[],
                fs(&[("/t/a", found.clone())]),
            );
            assert_eq!(
                p.actions,
                [Action::Collision {
                    wanted: d.clone(),
                    found
                }]
            );
            assert!(p.owned.is_empty());
        }
    }

    #[test]
    fn adopts_our_own_link_when_ownership_was_lost() {
        let d = desired("a", LinkMode::Symlink, "h");
        let p = plan(
            std::slice::from_ref(&d),
            &[],
            fs(&[("/t/a", link("/store/skill/a"))]),
        );
        assert_eq!(p.actions, [Action::Keep(d.clone())]);
        assert_eq!(p.owned, [owned_from(&d)]);
    }

    #[test]
    fn user_replaced_owned_link_is_disowned_not_touched() {
        let d = desired("a", LinkMode::Symlink, "h");
        let o = owned_from(&d);
        let p = plan(
            std::slice::from_ref(&d),
            std::slice::from_ref(&o),
            fs(&[("/t/a", Existing::Dir)]),
        );
        assert_eq!(
            p.actions,
            [
                Action::Disown {
                    owned: o,
                    found: Existing::Dir
                },
                Action::Collision {
                    wanted: d,
                    found: Existing::Dir
                }
            ]
        );
        assert!(p.owned.is_empty());
    }

    #[test]
    fn removes_owned_entries_no_longer_wanted() {
        let gone = owned_from(&desired("gone", LinkMode::Symlink, "h"));
        let vanished = owned_from(&desired("vanished", LinkMode::Symlink, "h"));
        let replaced = owned_from(&desired("replaced", LinkMode::Symlink, "h"));
        let p = plan(
            &[],
            &[gone.clone(), vanished, replaced.clone()],
            fs(&[
                ("/t/gone", link("/store/skill/gone")),
                ("/t/replaced", Existing::File),
            ]),
        );
        assert_eq!(
            p.actions,
            [
                Action::Remove(gone),
                Action::Disown {
                    owned: replaced,
                    found: Existing::File
                }
            ]
        );
        assert!(p.owned.is_empty());
    }

    #[test]
    fn replanning_after_apply_is_all_keep() {
        let ds = vec![
            desired("a", LinkMode::Symlink, "1"),
            desired("b", LinkMode::Copy, "2"),
        ];
        let first = plan(&ds, &[], fs(&[]));
        let after = fs(&[("/t/a", link("/store/skill/a")), ("/t/b", Existing::Dir)]);
        let second = plan(&ds, &first.owned, after);
        assert!(second.actions.iter().all(|a| matches!(a, Action::Keep(_))));
        assert_eq!(second.owned, first.owned);
    }
}

//! `loadout.lock` and `sync --locked`.

mod common;

use common::{Sandbox, skill};

fn read(p: &std::path::Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

#[test]
fn sync_pins_sources_and_items() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write("skills/a/SKILL.md", &skill("a", "A."));
    let c1 = repo.commit("one");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);

    let lock = read(&s.data.join("loadout.lock"));
    assert!(lock.contains(&c1), "{lock}");
    let mut st = s.insta();
    st.add_filter(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", "[TIME]");
    // TOML writes Windows paths as 'literal' strings.
    st.add_filter(r"url = '(.*)'", "url = \"$1\"");
    st.bind(|| insta::assert_snapshot!("lock", lock));

    // Unchanged upstream: the lock is byte-identical (fetched_at kept).
    s.ok(&["sync"]);
    assert_eq!(read(&s.data.join("loadout.lock")), lock);

    repo.write("skills/a/SKILL.md", &skill("a", "A2."));
    let c2 = repo.commit("two");
    s.ok(&["sync", "--yes"]);
    let lock2 = read(&s.data.join("loadout.lock"));
    assert!(lock2.contains(&c2) && !lock2.contains(&c1));
}

#[test]
fn locked_sync_on_another_machine_installs_the_pins() {
    let a = Sandbox::with_claude();
    let repo = a.source("team-skills");
    repo.write("skills/a/SKILL.md", &skill("a", "Version one."));
    repo.commit("one");
    a.ok(&["subscribe", &repo.url()]);
    a.ok(&["sync"]);

    // Upstream moves on after A synced.
    repo.write("skills/a/SKILL.md", &skill("a", "Version two."))
        .write("skills/b/SKILL.md", &skill("b", "New."));
    repo.commit("two");

    // Machine B: same config and lock, no checkouts yet.
    let b = Sandbox::with_claude();
    b.write_config(&read(&a.config.join("config.toml")));
    std::fs::create_dir_all(&b.data).unwrap();
    std::fs::copy(a.data.join("loadout.lock"), b.data.join("loadout.lock")).unwrap();
    b.ok(&["sync", "--locked"]);
    let installed = read(&b.skills_dir().join("a/SKILL.md"));
    assert!(installed.contains("Version one."), "{installed}");
    assert!(!b.skills_dir().join("b").exists());
    assert_eq!(
        read(&b.data.join("loadout.lock")),
        read(&a.data.join("loadout.lock"))
    );
}

#[test]
fn locked_sync_fails_when_lock_does_not_match() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write("skills/a/SKILL.md", &skill("a", "A."));
    repo.commit("one");
    s.ok(&["subscribe", &repo.url()]);
    s.ok(&["sync"]);
    let lock_path = s.data.join("loadout.lock");
    let good = read(&lock_path);

    // A tampered content hash.
    let bad = regex_replace_hash(&good);
    std::fs::write(&lock_path, &bad).unwrap();
    let out = s.run(&["sync", "--locked"]);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("content differs from loadout.lock"),
        "{}",
        out.stderr
    );
    assert_eq!(read(&lock_path), bad, "nothing changed");

    // A source missing from the lock.
    let other = s.source("other-skills");
    other.write("skills/o/SKILL.md", &skill("o", "O."));
    other.commit("o");
    std::fs::write(&lock_path, &good).unwrap();
    s.ok(&["subscribe", &other.url()]);
    let out = s.run(&["sync", "--locked"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("not in loadout.lock"), "{}", out.stderr);
    assert!(!s.skills_dir().join("o").exists());
}

fn regex_replace_hash(lock: &str) -> String {
    let i = lock.find("blake3:").unwrap() + "blake3:".len();
    let mut out = lock.to_owned();
    out.replace_range(i..i + 4, "dead");
    out
}

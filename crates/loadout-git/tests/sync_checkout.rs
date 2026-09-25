use loadout_git::fixture::{FixtureRepo, isolated_git};

#[test]
fn clones_then_follows_updates_and_discards_local_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let git = isolated_git(&tmp.path().join("scratch"));
    let origin = FixtureRepo::init(tmp.path().join("origin"), git.clone());
    origin.write("LOADOUT.md", "v1\n");
    let c1 = origin.commit("one");

    let checkout = tmp.path().join("repos/abc");
    let got = git.sync_checkout(&origin.url(), &checkout, None).unwrap();
    assert_eq!(got, c1);
    assert_eq!(
        std::fs::read_to_string(checkout.join("LOADOUT.md")).unwrap(),
        "v1\n"
    );

    origin
        .write("LOADOUT.md", "v2\n")
        .write("skills/a/SKILL.md", "a");
    let c2 = origin.commit("two");
    std::fs::write(checkout.join("stray.txt"), "junk").unwrap();
    std::fs::write(checkout.join("LOADOUT.md"), "edited").unwrap();

    let got = git.sync_checkout(&origin.url(), &checkout, None).unwrap();
    assert_eq!(got, c2);
    assert_eq!(
        std::fs::read_to_string(checkout.join("LOADOUT.md")).unwrap(),
        "v2\n"
    );
    assert!(checkout.join("skills/a/SKILL.md").is_file());
    assert!(!checkout.join("stray.txt").exists());

    // Idempotent when nothing changed upstream.
    assert_eq!(
        git.sync_checkout(&origin.url(), &checkout, None).unwrap(),
        c2
    );
}

#[test]
fn checks_out_named_ref() {
    let tmp = tempfile::tempdir().unwrap();
    let git = isolated_git(&tmp.path().join("scratch"));
    let origin = FixtureRepo::init(tmp.path().join("origin"), git.clone());
    origin.write("f", "main");
    origin.commit("main");
    git.run(Some(&origin.path), ["checkout", "--quiet", "-b", "stable"])
        .unwrap();
    origin.write("f", "stable");
    let stable = origin.commit("stable");
    git.run(Some(&origin.path), ["checkout", "--quiet", "main"])
        .unwrap();

    let checkout = tmp.path().join("co");
    let got = git
        .sync_checkout(&origin.url(), &checkout, Some("stable"))
        .unwrap();
    assert_eq!(got, stable);
    assert_eq!(
        std::fs::read_to_string(checkout.join("f")).unwrap(),
        "stable"
    );
}

#[test]
fn missing_repo_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let git = isolated_git(&tmp.path().join("scratch"));
    let err = git
        .sync_checkout(
            &tmp.path().join("nope").to_string_lossy(),
            &tmp.path().join("co"),
            None,
        )
        .unwrap_err();
    assert!(matches!(err, loadout_git::GitError::Failed { .. }), "{err}");
}

#[test]
fn can_read_reports_access() {
    let tmp = tempfile::tempdir().unwrap();
    let git = isolated_git(&tmp.path().join("scratch"));
    let origin = FixtureRepo::init(tmp.path().join("origin"), git.clone());
    origin.write("f", "x");
    origin.commit("one");
    assert!(git.can_read(&origin.url()).unwrap());
    assert!(
        git.can_read(&tmp.path().join("nope").to_string_lossy())
            .is_err()
    );
}

#[test]
fn fetch_without_checkout_then_pin_and_diff() {
    let tmp = tempfile::tempdir().unwrap();
    let git = isolated_git(&tmp.path().join("scratch"));
    let origin = FixtureRepo::init(tmp.path().join("origin"), git.clone());
    origin.write("LOADOUT.md", "v1\n");
    let c1 = origin.commit("one");
    let dir = tmp.path().join("repos/x");
    assert_eq!(git.sync_checkout(&origin.url(), &dir, None).unwrap(), c1);

    origin.write("LOADOUT.md", "v2\n");
    let c2 = origin.commit("two");
    // Fetching leaves the working tree at the applied commit.
    assert_eq!(git.fetch(&origin.url(), &dir, None).unwrap(), c2);
    assert_eq!(git.head(&dir).unwrap(), c1);
    assert_eq!(
        std::fs::read_to_string(dir.join("LOADOUT.md")).unwrap(),
        "v1\n"
    );
    assert!(git.has_commit(&dir, &c2));

    let diff = git.diff(&dir, &c1, &c2).unwrap();
    assert!(diff.contains("-v1") && diff.contains("+v2"), "{diff}");

    git.checkout(&dir, &c2).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("LOADOUT.md")).unwrap(),
        "v2\n"
    );
    git.checkout(&dir, &c1).unwrap();
    assert_eq!(git.head(&dir).unwrap(), c1);
}

#[test]
fn fetches_a_pinned_commit_on_a_fresh_machine() {
    let tmp = tempfile::tempdir().unwrap();
    let git = isolated_git(&tmp.path().join("scratch"));
    let origin = FixtureRepo::init(tmp.path().join("origin"), git.clone());
    origin.write("f", "1");
    let c1 = origin.commit("one");
    origin.write("f", "2");
    origin.commit("two");

    let dir = tmp.path().join("repos/fresh");
    assert!(!git.has_commit(&dir, &c1));
    git.fetch_commit(&origin.url(), &dir, &c1).unwrap();
    assert!(git.has_commit(&dir, &c1));
    git.checkout(&dir, &c1).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("f")).unwrap(), "1");
}

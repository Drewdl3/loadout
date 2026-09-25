//! The end-to-end onboarding story in `docs/onboarding.md` and
//! `examples/onboard.sh`: a squad lead sets up a new squad's catalog from
//! the org's template, registers it in the company config, and a new developer
//! joins, adds a personal layer and reviews an update.

mod common;

use common::{Sandbox, publish_examples};
use serde_json::Value;

const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "Sam Lead"),
    ("GIT_AUTHOR_EMAIL", "sam@example.com"),
    ("GIT_COMMITTER_NAME", "Sam Lead"),
    ("GIT_COMMITTER_EMAIL", "sam@example.com"),
];

/// A Payments developer's environment (membership rules use env vars).
fn env(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    let mut v = vec![
        ("ACME_ORG", "eng"),
        ("ACME_TEAM", "payments-dev"),
        ("ACME_ROLE", "developer"),
    ];
    v.extend_from_slice(&IDENTITY);
    v.extend_from_slice(extra);
    v
}

fn ok(s: &Sandbox, args: &[&str], extra: &[(&'static str, &'static str)]) -> String {
    let out = s.run_env(args, &env(extra));
    assert!(
        out.code == 0,
        "lo {args:?} exited {}\n{}\n{}",
        out.code,
        out.stdout,
        out.stderr
    );
    out.stdout
}

fn json(s: &Sandbox, args: &[&str]) -> Value {
    let mut a = args.to_vec();
    a.push("--json");
    serde_json::from_str(&ok(s, &a, &[])).unwrap()
}

fn commit(s: &Sandbox, dir: &std::path::Path, msg: &str) {
    s.git.run(Some(dir), ["add", "--all"]).unwrap();
    s.git
        .run(Some(dir), ["commit", "--quiet", "-m", msg])
        .unwrap();
}

#[test]
fn a_new_squad_and_a_new_developer_onboard() {
    // Acme's company config and group repos already exist (examples/).
    let sam = Sandbox::with_claude();
    let company_config = publish_examples(&sam);
    let remotes = sam.root.join("remotes");

    // ── 1. Sam, the Checkout squad's lead, is already set up. ────────────
    ok(&sam, &["init", &company_config, "--non-interactive"], &[]);
    let templates = json(&sam, &["template", "list"]);
    assert_eq!(
        templates["templates"][0]["id"],
        "eng-skills:template/pr-workflow"
    );

    // ── 2. Sam creates the squad's catalog, building on Payments. ────────
    let squad = remotes.join("checkout-squad");
    ok(
        &sam,
        &[
            "init",
            "--new-source",
            squad.to_str().unwrap(),
            "--layer",
            "squad",
            "--group",
            "checkout",
            "--upstream",
            "../payments-skills",
            "--git-init",
        ],
        &[],
    );
    // ...and turns Engineering's PR template into the squad's own skill.
    let used = json(
        &sam,
        &[
            "template",
            "use",
            "pr-workflow",
            "--dir",
            squad.to_str().unwrap(),
            "--name",
            "checkout-prs",
            "--set",
            "team=Checkout",
            "--set",
            "reviewers=1",
        ],
    );
    assert_eq!(used["id"], "checkout-squad:skill/checkout-prs");
    std::fs::remove_dir_all(squad.join("skills/example")).unwrap();
    commit(
        &sam,
        &squad,
        "Add our PR workflow from the Engineering template",
    );
    let audit = sam.run(&["audit", "--path", squad.to_str().unwrap()]);
    assert_eq!(audit.code, 0, "{}", audit.stdout);

    // ── 3. The squad is registered in the company config (a reviewed PR to the
    //       company config repo in real life). ────────────────────────────────────
    let company_config_dir = remotes.join("acme-config");
    let manifest = std::fs::read_to_string(company_config_dir.join("LOADOUT.md")).unwrap();
    let group = format!(
        "    - layer: squad\n      name: checkout\n      sources: [{:?}]\n      membership: {{ manual: true }}\n  policy:",
        squad.to_str().unwrap()
    );
    std::fs::write(
        company_config_dir.join("LOADOUT.md"),
        manifest.replacen("  policy:", &group, 1),
    )
    .unwrap();
    commit(&sam, &company_config_dir, "Register the Checkout squad");

    // ── 4. Alex joins Acme and the Checkout squad. ───────────────────────
    let alex = Sandbox::with_claude();
    ok(&alex, &["init", &company_config, "--non-interactive"], &[]);
    let profile = json(&alex, &["profile"]);
    let checkout = profile["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["group"] == "checkout")
        .expect("the company config lists the squad")
        .clone();
    assert_eq!(checkout["member"], false);
    // Joining installs the squad's items right away.
    ok(&alex, &["join", "squad:checkout"], &[]);
    let prs = std::fs::read_to_string(alex.skills_dir().join("checkout-prs/SKILL.md")).unwrap();
    assert!(prs.contains("# Pull requests in Checkout"), "{prs}");
    assert!(
        prs.contains("Approvals required before merging: 1 "),
        "{prs}"
    );
    // The company's `loadout` skill teaches Alex's AI tools the CLI.
    assert!(alex.skills_dir().join("loadout/SKILL.md").exists());
    let why = json(&alex, &["why", "skill/checkout-prs"]);
    assert_eq!(why["winner"], "checkout-squad:skill/checkout-prs");

    // ── 5. Alex adds a personal layer on top of the squad. ───────────────
    let mine = alex.root.join("alex-skills");
    ok(
        &alex,
        &[
            "init",
            "--new-source",
            mine.to_str().unwrap(),
            "--layer",
            "user",
            "--upstream",
            squad.to_str().unwrap(),
            "--subscribe",
        ],
        &[("USER", "alex")],
    );
    std::fs::create_dir_all(mine.join("skills/write-spec")).unwrap();
    std::fs::write(
        mine.join("skills/write-spec/SKILL.md"),
        "---\nname: write-spec\ndescription: Alex's spec format.\n---\nMy way.\n",
    )
    .unwrap();
    commit(&alex, &mine, "My spec format");
    ok(&alex, &["sync", "--yes"], &[]);
    let why = json(&alex, &["why", "skill/write-spec"]);
    assert_eq!(why["winner"], "alex-skills:skill/write-spec");

    // ── 6. Sam updates the squad's skill; Alex reviews it. ───────────────
    let prs_path = squad.join("skills/checkout-prs/SKILL.md");
    let updated = std::fs::read_to_string(&prs_path)
        .unwrap()
        .replace("merging: 1 ", "merging: 2 ");
    std::fs::write(&prs_path, updated).unwrap();
    commit(&sam, &squad, "Two approvals from now on");
    let sync = alex.run_env(&["sync"], &env(&[]));
    assert_eq!(
        sync.code, 2,
        "squad changes wait for review\n{}",
        sync.stdout
    );
    let diff = alex.run_env(&["diff", "checkout-squad"], &env(&[]));
    assert_eq!(diff.code, 2, "diff exits 2 while changes are pending");
    assert!(
        diff.stdout
            .contains("+4. Approvals required before merging: 2"),
        "{}",
        diff.stdout
    );
    ok(&alex, &["approve", "checkout-squad"], &[]);
    let prs = std::fs::read_to_string(alex.skills_dir().join("checkout-prs/SKILL.md")).unwrap();
    assert!(
        prs.contains("Approvals required before merging: 2 "),
        "{prs}"
    );
}

//! The end-to-end onboarding story in `docs/onboarding.md` and
//! `examples/onboard.sh`, bottom-up: a squad lead connects to the team's
//! source and builds the squad's on top of it, a new developer connects
//! with the squad's link, adds a personal layer and reviews an update, and
//! a company config arrives later.

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
fn a_squad_and_a_developer_onboard_bottom_up() {
    // Acme's team repos already exist (examples/); the company config comes
    // later.
    let sam = Sandbox::with_claude();
    let company_config = publish_examples(&sam);
    let remotes = sam.root.join("remotes");
    let payments = remotes.join("payments-skills");

    // ── 1. Sam connects to the Payments team's source. ───────────────────
    let init = json(
        &sam,
        &["init", payments.to_str().unwrap(), "--non-interactive"],
    );
    assert_eq!(init["kind"], "source");
    let names: Vec<&str> = init["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["payments-skills", "eng-skills"], "and its upstream");
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

    // ── 3. Alex connects with the squad's link: squad, team and org. ─────
    let alex = Sandbox::with_claude();
    let init = json(
        &alex,
        &["init", squad.to_str().unwrap(), "--non-interactive"],
    );
    assert_eq!(init["kind"], "source");
    assert_eq!(init["sources"].as_array().unwrap().len(), 3);
    let prs = std::fs::read_to_string(alex.skills_dir().join("checkout-prs/SKILL.md")).unwrap();
    assert!(prs.contains("# Pull requests in Checkout"), "{prs}");
    assert!(
        prs.contains("Approvals required before merging: 1 "),
        "{prs}"
    );
    // Payments' write-spec (team) beats Engineering's (org).
    let why = json(&alex, &["why", "skill/write-spec"]);
    assert_eq!(why["winner"], "payments-skills:skill/write-spec");

    // ── 4. Alex adds a personal layer on top of the squad. ───────────────
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

    // ── 5. Sam updates the squad's skill; Alex reviews it. ───────────────
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

    // ── 6. Later, Acme adds a company config that lists the squad. ───────
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
    commit(&sam, &company_config_dir, "List the Checkout squad");
    let init = json(&alex, &["init", &company_config, "--non-interactive"]);
    assert_eq!(init["kind"], "company_config");
    let profile = json(&alex, &["profile"]);
    let member = |layer: &str, group: &str| {
        profile["groups"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["layer"] == layer && u["group"] == group && u["member"] == true)
    };
    assert!(member("org", "eng") && member("team", "payments-dev"));
    // Alex keeps the squad and personal sources, and gets the company's
    // items: its locked standards, and the `loadout` skill for AI tools.
    assert!(alex.skills_dir().join("checkout-prs/SKILL.md").exists());
    assert!(alex.skills_dir().join("loadout/SKILL.md").exists());
    let why = json(&alex, &["why", "skill/code-review-standards"]);
    assert_eq!(why["winner"], "acme-config:skill/code-review-standards");
    let why = json(&alex, &["why", "skill/write-spec"]);
    assert_eq!(why["winner"], "alex-skills:skill/write-spec");
}

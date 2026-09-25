//! `lo init --new-source` / `--new-company` and the `user`
//! layer for personal sources.

mod common;

use common::{Sandbox, skill};
use loadout_git::fixture::FixtureRepo;
use serde_json::Value;

const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "Jane Doe"),
    ("GIT_AUTHOR_EMAIL", "jane@example.com"),
    ("GIT_COMMITTER_NAME", "Jane Doe"),
    ("GIT_COMMITTER_EMAIL", "jane@example.com"),
];

fn run_json(s: &Sandbox, args: &[&str], env: &[(&str, &str)]) -> Value {
    let mut all = args.to_vec();
    all.push("--json");
    let mut vars = IDENTITY.to_vec();
    vars.extend_from_slice(env);
    let out = s.run_env(&all, &vars);
    assert_eq!(
        out.code, 0,
        "lo {args:?}\nstdout:\n{}\nstderr:\n{}",
        out.stdout, out.stderr
    );
    serde_json::from_str(&out.stdout).unwrap()
}

fn path(p: &std::path::Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn init_without_arguments_explains_the_choices() {
    let s = Sandbox::new();
    let out = s.run(&["init"]);
    assert_eq!(out.code, 1);
    for hint in [
        "<company-config-url>",
        "--new-source",
        "--new-company",
        "--project",
    ] {
        assert!(out.stderr.contains(hint), "{}", out.stderr);
    }
    let out = s.run(&["init", "--project", "--new-source"]);
    assert_eq!(out.code, 1);
}

#[test]
fn new_company_config_is_a_working_company_config() {
    let s = Sandbox::with_claude();
    let dir = s.root.join("remotes/acme-config");
    let r = run_json(
        &s,
        &[
            "init",
            "--new-company",
            path(&dir),
            "--company",
            "acme",
            "--git-init",
        ],
        &[],
    );
    assert_eq!(r["created"]["company_config"], true);
    assert_eq!(r["created"]["layer"], "company");
    assert_eq!(r["created"]["group"], "acme");
    assert_eq!(r["committed"], true);
    assert!(dir.join(".git").exists());
    let agents = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    assert!(agents.contains("company **company config**"), "{agents}");
    assert!(agents.contains("`company.groups`"), "{agents}");

    // Someone joins it.
    let init = s.json(&["init", path(&dir), "--non-interactive"]);
    assert_eq!(init["company"], "acme");
    assert_eq!(init["sync"]["sources"][0]["name"], "acme-config");
}

#[test]
fn new_source_with_upstream_is_not_committed_by_default() {
    let s = Sandbox::new();
    let dir = s.root.join("checkout-squad");
    let r = run_json(
        &s,
        &[
            "init",
            "--catalog",
            path(&dir),
            "--layer",
            "squad",
            "--group",
            "checkout",
            "--upstream",
            "../eng-skills",
            "--upstream",
            "https://git.example.com/acme/company-config",
        ],
        &[],
    );
    assert_eq!(r["created"]["name"], "checkout-squad");
    assert_eq!(r["committed"], false);
    assert!(!dir.join(".git").exists());
    let manifest = std::fs::read_to_string(dir.join("LOADOUT.md")).unwrap();
    let doc = loadout_model::ManifestDoc::parse(&manifest).unwrap();
    assert_eq!(doc.manifest.layer, "squad");
    assert_eq!(doc.manifest.group.as_deref(), Some("checkout"));
    assert_eq!(doc.manifest.upstream.len(), 2);
    assert_eq!(doc.manifest.upstream[0].url(), "../eng-skills");
    // Instructions for agents, with this repo's names.
    let agents = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    assert!(
        agents.starts_with("# AGENTS.md — checkout-squad"),
        "{agents}"
    );
    assert!(agents.contains("`squad:checkout`"), "{agents}");
    assert!(agents.contains("`{{variable}}`"), "{agents}");
    assert!(!agents.contains("company.groups"), "{agents}");

    // Same thing through `new-source`; refuses to overwrite.
    let again = s.run(&["new-source", path(&dir)]);
    assert_eq!(again.code, 1);
}

#[test]
fn personal_source_builds_on_the_team_and_outranks_it() {
    let s = Sandbox::with_claude();
    let team = FixtureRepo::init(s.root.join("remotes/payments-skills"), s.git.clone());
    team.write(
        "LOADOUT.md",
        "---\nloadout: 1\nname: payments-skills\nlayer: team\ngroup: payments-dev\n---\n",
    )
    .write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "Team spec."),
    )
    .write("skills/review/SKILL.md", &skill("review", "Team review."));
    team.commit("team");

    let mine = s.root.join("my-skills");
    let r = run_json(
        &s,
        &[
            "init",
            "--new-source",
            path(&mine),
            "--layer",
            "user",
            "--upstream",
            &team.url(),
            "--subscribe",
        ],
        &[("USER", "Jane Doe")],
    );
    assert_eq!(r["created"]["group"], "jane-doe");
    assert_eq!(r["committed"], true);
    let sync = &r["sync"];
    let names: Vec<&str> = sync["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["my-skills", "payments-skills"]);
    let skills = s.skills_dir();
    assert!(skills.join("review/SKILL.md").exists());

    // A personal override wins over the team's (user outranks team).
    std::fs::create_dir_all(mine.join("skills/write-spec")).unwrap();
    std::fs::write(
        mine.join("skills/write-spec/SKILL.md"),
        skill("write-spec", "My spec."),
    )
    .unwrap();
    s.git.run(Some(&mine), ["add", "--all"]).unwrap();
    s.git
        .run(Some(&mine), ["commit", "--quiet", "-m", "mine"])
        .unwrap();
    s.ok(&["sync", "--yes"]);
    let installed = std::fs::read_to_string(skills.join("write-spec/SKILL.md")).unwrap();
    assert!(installed.contains("My spec."), "{installed}");
    let why = s.json(&["why", "skill/write-spec"]);
    assert_eq!(why["winner"], "my-skills:skill/write-spec");
}

#[test]
fn bad_names_fail_before_anything_is_created() {
    let s = Sandbox::new();
    let dir = s.root.join("remotes/new-one");
    let out = s.run(&["init", "--new-source", path(&dir), "--name", "Drewdles"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("kebab-case"), "{}", out.stderr);
    assert!(!dir.exists(), "the directory must not be created");
    let out = s.run(&["init", "--new-company", path(&dir), "--company", "Acme Co"]);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("company must be kebab-case"),
        "{}",
        out.stderr
    );
    assert!(!dir.exists());
}

#[test]
fn local_paths_that_are_not_git_repos_are_explained() {
    let s = Sandbox::new();
    // Missing folder.
    let missing = s.root.join("nowhere");
    let out = s.run(&["subscribe", path(&missing)]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("no folder at"), "{}", out.stderr);
    // A plain folder, and a folder inside a repo.
    let plain = s.root.join("plain");
    std::fs::create_dir_all(plain.join("inner")).unwrap();
    let out = s.run(&["subscribe", path(&plain), "--dry-run"]);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("is not a Git repository"),
        "{}",
        out.stderr
    );
    let repo = s.source("repo");
    repo.write("sub/skills/a/SKILL.md", &skill("a", "A."));
    repo.commit("init");
    let sub = std::path::Path::new(&repo.url()).join("sub");
    let out = s.run(&["subscribe", path(&sub)]);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("is inside the repository at"),
        "{}",
        out.stderr
    );
    // Nothing was subscribed along the way; the repo itself works.
    let out = s.run(&["subscribe", path(&sub.join(".."))]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let config = std::fs::read_to_string(s.config.join("config.toml")).unwrap();
    assert!(!config.contains("sub"), "{config}");
}

#[test]
fn company_config_with_its_own_layers_ranks_a_custom_layer() {
    let s = Sandbox::with_claude();
    let company_config = s.root.join("remotes/acme-config");
    let rust = s.root.join("remotes/rust-chapter");
    let layers = "company=0,chapter=15,user=50";
    run_json(
        &s,
        &[
            "init",
            "--new-company",
            path(&company_config),
            "--company",
            "acme",
            "--layers",
            layers,
            "--git-init",
        ],
        &[],
    );
    let manifest = std::fs::read_to_string(company_config.join("LOADOUT.md")).unwrap();
    assert!(
        manifest.contains("- { name: chapter, rank: 15 }"),
        "{manifest}"
    );
    assert!(!manifest.contains("name: team"), "{manifest}");
    run_json(
        &s,
        &[
            "init",
            "--new-source",
            path(&rust),
            "--layer",
            "chapter",
            "--group",
            "rustaceans",
            "--git-init",
        ],
        &[],
    );
    s.json(&["init", path(&company_config), "--non-interactive"]);
    s.ok(&["subscribe", path(&rust)]);
    s.ok(&["enable", "skill/example"]);
    s.ok(&["sync"]);
    // Both repos have skill/example; the `chapter` layer (rank 15) outranks
    // `company` (0).
    let list = s.json(&["list"]);
    let example = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == "rust-chapter:skill/example")
        .unwrap_or_else(|| panic!("{list:#}"));
    assert_eq!(example["layer"], "chapter");

    // Bad layer lists are refused before anything is created.
    let other = s.root.join("remotes/other");
    for bad in [
        "chapter=15",
        "company=0,team",
        "company=0,team=0",
        "company=0,Team=5",
    ] {
        let out = s.run(&["init", "--new-company", path(&other), "--layers", bad]);
        assert_eq!(out.code, 1, "{bad}");
        assert!(!other.exists(), "{bad}");
    }
}

//! Templates: a higher source publishes
//! `templates/<name>/`, a lower group turns one into its own skill.

mod common;

use common::{Sandbox, skill};
use loadout_git::fixture::FixtureRepo;
use serde_json::Value;

const TEMPLATE: &str = "---
name: pr-workflow
description: How {{team}} works with pull requests.
template:
  description: A pull request workflow skill; fill in your squad's conventions.
  variables:
    - { name: team, description: Your team or squad }
    - { name: reviewers, description: How many approvals, default: \"2\" }
loadout:
  tags: [git, prs]
---
# PRs in {{team}}

Every PR needs {{ reviewers }} approvals. See checklist.md.
";

struct World {
    s: Sandbox,
    squad: FixtureRepo,
}

/// eng (org) publishes a template; the user subscribes to the checkout
/// squad's source, which builds on eng.
fn world() -> World {
    let s = Sandbox::with_claude();
    let eng = FixtureRepo::init(s.root.join("remotes/eng-skills"), s.git.clone());
    eng.write(
        "LOADOUT.md",
        "---\nloadout: 1\nname: eng-skills\nlayer: org\ngroup: eng\n---\n",
    )
    .write("templates/pr-workflow/SKILL.md", TEMPLATE)
    .write(
        "templates/pr-workflow/checklist.md",
        "- [ ] {{team}} changelog entry\n",
    )
    .write(
        "templates/broken/SKILL.md",
        "---\ntemplate: { variables: [{ name: 1bad }] }\n---\n",
    )
    .write("skills/style/SKILL.md", &skill("style", "Eng style."));
    eng.commit("eng");
    let squad = FixtureRepo::init(s.root.join("remotes/checkout-squad"), s.git.clone());
    squad
        .write(
            "LOADOUT.md",
            "---\nloadout: 1\nname: checkout-squad\nlayer: squad\ngroup: checkout\nupstream: [../eng-skills]\n---\n",
        )
        .write("skills/deploy/SKILL.md", &skill("deploy", "Deploy."));
    squad.commit("squad");
    s.ok(&["subscribe", &squad.url()]);
    s.ok(&["sync"]);
    World { s, squad }
}

#[test]
fn templates_are_listed_but_never_installed() {
    let w = world();
    let out = w.s.run(&["template", "list", "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let list: Value = serde_json::from_str(&out.stdout).unwrap();
    let t = &list["templates"][0];
    assert_eq!(list["templates"].as_array().unwrap().len(), 1, "{list:#}");
    assert_eq!(t["id"], "eng-skills:template/pr-workflow");
    assert_eq!(t["variables"][0]["name"], "team");
    assert_eq!(t["variables"][1]["default"], "2");
    assert_eq!(t["tags"], serde_json::json!(["git", "prs"]));
    assert_eq!(t["files"], serde_json::json!(["SKILL.md", "checklist.md"]));
    assert!(list["warnings"].to_string().contains("templates/broken"));
    // Only real skills are installed.
    assert!(w.s.skills_dir().join("style").exists());
    assert!(!w.s.skills_dir().join("pr-workflow").exists());

    let show = w.s.json(&["template", "show", "pr-workflow"]);
    assert!(show["text"].as_str().unwrap().contains("{{team}}"));
    let human = w.s.ok(&["template"]).stdout;
    assert!(human.contains("eng-skills:template/pr-workflow"), "{human}");
    assert!(human.contains("variables: team, reviewers"), "{human}");
}

#[test]
fn use_writes_a_real_skill_that_syncs() {
    let w = world();
    // Missing a required value (no terminal to ask): refused.
    let out = w.s.run(&[
        "template",
        "use",
        "pr-workflow",
        "--dir",
        w.squad.path.to_str().unwrap(),
    ]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("team"), "{}", out.stderr);
    let out = w.s.run(&[
        "template",
        "use",
        "pr-workflow",
        "--dir",
        w.squad.path.to_str().unwrap(),
        "--set",
        "nope=1",
    ]);
    assert!(
        out.stderr.contains("no variable \"nope\""),
        "{}",
        out.stderr
    );

    // Into a local checkout of the squad's repo.
    let r = w.s.json(&[
        "template",
        "use",
        "eng-skills:template/pr-workflow",
        "--dir",
        w.squad.path.to_str().unwrap(),
        "--name",
        "checkout-prs",
        "--set",
        "team=Checkout",
    ]);
    assert_eq!(r["id"], "checkout-squad:skill/checkout-prs");
    assert_eq!(r["values"]["reviewers"], "2");
    assert_eq!(r["working_clone"], false);
    let dir = w.squad.path.join("skills/checkout-prs");
    let main = std::fs::read_to_string(dir.join("SKILL.md")).unwrap();
    assert!(
        main.starts_with("---\nname: \"checkout-prs\"\ndescription: How Checkout works"),
        "{main}"
    );
    assert!(!main.contains("\ntemplate:"), "{main}");
    assert!(
        main.contains("from_template: \"eng-skills:template/pr-workflow@"),
        "{main}"
    );
    assert!(main.contains("Every PR needs 2 approvals."), "{main}");
    assert_eq!(
        std::fs::read_to_string(dir.join("checklist.md")).unwrap(),
        "- [ ] Checkout changelog entry\n"
    );
    // The same name again is refused.
    let again = w.s.run(&[
        "template",
        "use",
        "pr-workflow",
        "--dir",
        w.squad.path.to_str().unwrap(),
        "--name",
        "checkout-prs",
        "--set",
        "team=x",
    ]);
    assert!(again.stderr.contains("already exists"), "{}", again.stderr);

    // Committed and synced, it's an ordinary skill of the squad.
    w.squad.commit("Add checkout-prs from template");
    w.s.ok(&["sync", "--yes"]);
    let installed =
        std::fs::read_to_string(w.s.skills_dir().join("checkout-prs/SKILL.md")).unwrap();
    assert!(installed.contains("# PRs in Checkout"));
    let list = w.s.json(&["list"]);
    assert!(
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == "checkout-squad:skill/checkout-prs"),
        "{list:#}"
    );
    let audit =
        w.s.run(&["audit", "--path", w.squad.path.to_str().unwrap()]);
    assert_eq!(audit.code, 0, "{}", audit.stdout);
}

#[test]
fn use_into_a_source_writes_its_working_clone() {
    let w = world();
    let r = w.s.json(&[
        "template",
        "use",
        "pr-workflow",
        "--into",
        "checkout-squad",
        "--set",
        "team=Checkout",
    ]);
    assert_eq!(r["working_clone"], true);
    let path = std::path::PathBuf::from(r["path"].as_str().unwrap());
    assert!(path.join("SKILL.md").exists());
    // The working clone is under data/work, not the squad's repo itself.
    assert!(
        path.starts_with(w.s.data.join("work")),
        "{}",
        path.display()
    );
    assert!(!w.squad.path.join("skills/pr-workflow").exists());
    let human = w.s.run(&[
        "template",
        "use",
        "pr-workflow",
        "--into",
        "checkout-squad",
        "--name",
        "other",
        "--set",
        "team=x",
    ]);
    assert!(
        human.stdout.contains("lo export-source checkout-squad"),
        "{}",
        human.stdout
    );
}

//! `lo skills`: every skill on the computer, and managing the ones Loadout
//! didn't install. The setup mirrors `npx skills`: real folders in
//! `~/.agents/skills`, links to them from other tools, a lock file naming
//! the repo they came from.

mod common;

use std::path::{Path, PathBuf};

use common::{Sandbox, skill};
use loadout_git::fixture::FixtureRepo;
use serde_json::Value;

/// A plain skills repo (no `LOADOUT.md`), the way people publish skills.
fn plain_repo(s: &Sandbox) -> FixtureRepo {
    let r = FixtureRepo::init(s.root.join("remotes/acme-skills"), s.git.clone());
    r.write("README.md", "# Acme skills\n");
    r.write(
        "skills/engineering/tdd/SKILL.md",
        &skill("tdd", "Test first."),
    );
    r.write("skills/engineering/tdd/tests.md", "How to test.\n");
    r.write(
        "skills/writing/grill-me/SKILL.md",
        &skill("grill-me", "Ask hard questions."),
    );
    r.write(
        "skills/writing/extra/SKILL.md",
        &skill("extra", "Not installed here."),
    );
    r.commit("skills");
    r
}

/// Writes a skill folder as `npx skills` installed it.
fn installed(dir: &Path, name: &str, description: &str, extra: Option<(&str, &str)>) {
    std::fs::create_dir_all(dir.join(name)).unwrap();
    std::fs::write(dir.join(name).join("SKILL.md"), skill(name, description)).unwrap();
    if let Some((f, text)) = extra {
        std::fs::write(dir.join(name).join(f), text).unwrap();
    }
}

#[cfg(unix)]
fn link(from: &Path, to: &str) {
    std::fs::create_dir_all(from.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(to, from).unwrap();
}

/// Claude Code, Pi and Codex installed. `npx skills` put tdd and grill-me
/// in `~/.agents/skills` (Codex's folder) and linked them for Pi; Claude
/// Code has a hand-written skill of its own.
#[cfg(unix)]
fn world() -> (Sandbox, FixtureRepo) {
    let s = Sandbox::with_claude();
    std::fs::create_dir_all(s.home.join(".codex")).unwrap();
    let repo = plain_repo(&s);
    let agents = s.home.join(".agents/skills");
    installed(
        &agents,
        "tdd",
        "Test first.",
        Some(("tests.md", "How to test.\n")),
    );
    installed(&agents, "grill-me", "Ask hard questions.", None);
    let pi = s.home.join(".pi/agent/skills");
    link(&pi.join("tdd"), "../../../.agents/skills/tdd");
    link(&pi.join("grill-me"), "../../../.agents/skills/grill-me");
    let lock = serde_json::json!({
        "version": 3,
        "skills": {
            "tdd": {"source": "acme/skills", "sourceType": "github", "sourceUrl": repo.url(), "skillPath": "skills/engineering/tdd/SKILL.md"},
            "grill-me": {"source": "acme/skills", "sourceType": "github", "sourceUrl": repo.url(), "skillPath": "skills/writing/grill-me/SKILL.md"}
        }
    });
    std::fs::write(s.home.join(".agents/.skill-lock.json"), lock.to_string()).unwrap();
    installed(&s.skills_dir(), "my-notes", "How I take notes.", None);
    (s, repo)
}

fn skill_named<'a>(inv: &'a Value, name: &str) -> &'a Value {
    inv["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name)
        .unwrap_or_else(|| panic!("no {name} in {inv:#}"))
}

fn tools(skill: &Value) -> Vec<String> {
    let mut t: Vec<String> = skill["copies"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["seen_by"].as_array().unwrap().clone())
        .map(|v| v["tool"].as_str().unwrap().to_owned())
        .collect();
    t.sort();
    t.dedup();
    t
}

fn store(s: &Sandbox, p: PathBuf) -> bool {
    std::fs::read_link(&p).is_ok_and(|t| t.starts_with(s.data.join("store")))
}

#[cfg(unix)]
#[test]
fn lists_every_skill_with_where_it_came_from_and_what_to_fix() {
    let (s, _) = world();
    let inv = s.json(&["skills"]);
    let tdd = skill_named(&inv, "tdd");
    assert_eq!(tdd["state"], "unmanaged", "{tdd:#}");
    // One folder, reached by Codex directly and by Pi through a link.
    assert_eq!(tdd["copies"].as_array().unwrap().len(), 1);
    assert_eq!(tools(tdd), ["codex", "pi"]);
    assert_eq!(tdd["copies"][0]["origin"]["source"], "acme/skills");
    assert_eq!(tdd["copies"][0]["origin"]["installer"], "npx skills");
    assert_eq!(
        tdd["issues"],
        serde_json::json!([{"kind": "missing", "tools": ["claude-code"]}])
    );
    let notes = skill_named(&inv, "my-notes");
    assert!(notes["copies"][0]["origin"].is_null());

    // A project's own copy of tdd: Claude Code would see both.
    let project = s.root.join("work/app");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    installed(
        &project.join(".claude/skills"),
        "my-notes",
        "Project notes.",
        None,
    );
    let inv = s.json(&["skills", "list", "--in", project.to_str().unwrap()]);
    let notes = skill_named(&inv, "my-notes");
    let kinds: Vec<&str> = notes["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["shadowed", "copies", "missing"], "{notes:#}");
    assert_eq!(notes["issues"][1]["differ"], true);
    let issues = s.json(&[
        "skills",
        "list",
        "--issues",
        "--in",
        project.to_str().unwrap(),
    ]);
    assert_eq!(issues["skills"].as_array().unwrap().len(), 3);

    let text = s.ok(&["skills"]).stdout;
    assert!(
        text.contains("not managed (from acme/skills via npx skills)"),
        "{text}"
    );
    assert!(text.contains("lo skills manage"), "{text}");
}

#[cfg(unix)]
#[test]
fn managing_follows_the_repo_or_copies_and_leaves_one_copy() {
    let (s, _) = world();
    let r = s.json(&["skills", "manage", "--all"]);
    let how = |n: &str| {
        r["managed"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == n)
            .unwrap_or_else(|| panic!("{n} in {r:#}"))
            .clone()
    };
    assert_eq!(how("tdd")["how"], "followed");
    assert_eq!(how("tdd")["source"], "acme-skills");
    assert_eq!(how("grill-me")["how"], "followed");
    assert_eq!(how("my-notes")["how"], "copied");
    assert_eq!(how("my-notes")["source"], "local-skills");
    assert_eq!(r["handed_over"].as_array().unwrap().len(), 3, "{r:#}");

    // Every tool now has Loadout's copy, and only that.
    let agents = s.home.join(".agents/skills");
    for p in [
        agents.join("tdd"),
        s.home.join(".pi/agent/skills/tdd"),
        s.skills_dir().join("tdd"),
        s.skills_dir().join("my-notes"),
        agents.join("my-notes"),
    ] {
        assert!(store(&s, p.clone()), "{} isn't Loadout's", p.display());
    }
    // The repo's other skills aren't installed.
    assert!(!s.skills_dir().join("extra").exists());
    let inv = s.json(&["skills"]);
    for name in ["tdd", "grill-me", "my-notes"] {
        let sk = skill_named(&inv, name);
        assert_eq!(sk["state"], "managed", "{sk:#}");
        assert_eq!(sk["issues"], serde_json::json!([]), "{sk:#}");
    }
    assert!(skill_named(&inv, "tdd")["item"] == "acme-skills:skill/tdd");
    assert_eq!(skill_named(&inv, "tdd")["handed_over"], true);
    // The originals are kept.
    assert!(s.data.join("adopted/tdd/tests.md").is_file());

    // Nothing left to manage.
    let out = s.run(&["skills", "manage", "tdd", "--json"]);
    assert_eq!(out.code, 1);
    assert!(out.stdout.contains("already managed"), "{}", out.stdout);
}

#[cfg(unix)]
#[test]
fn copying_instead_of_following_and_editing_your_local_copy() {
    let (s, _) = world();
    let r = s.json(&["skills", "manage", "tdd", "--copy"]);
    assert_eq!(r["managed"][0]["how"], "copied");
    let local = s.data.join("local/skills/tdd");
    assert!(local.join("tests.md").is_file());

    // Edit it in place, then save: no pull request, installed at once.
    std::fs::write(local.join("SKILL.md"), skill("tdd", "Test first, always.")).unwrap();
    let r = s.json(&["export-source", "local-skills", "-m", "Sharpen tdd"]);
    assert_eq!(r["applied"], true, "{r:#}");
    assert!(r["pull_request"].is_null());
    let installed = std::fs::read_to_string(s.skills_dir().join("tdd/SKILL.md")).unwrap();
    assert!(installed.contains("Test first, always."), "{installed}");
    let log = s
        .git
        .run(Some(&s.data.join("local")), ["log", "--format=%s"])
        .unwrap();
    assert!(log.starts_with("Sharpen tdd\nManage tdd\n"), "{log}");
}

#[cfg(unix)]
#[test]
fn unmanaging_puts_the_originals_back() {
    let (s, _) = world();
    s.ok(&["skills", "manage", "tdd", "my-notes"]);
    let r = s.json(&["skills", "unmanage", "tdd"]);
    assert_eq!(
        r["restored"][0]["path"],
        s.home.join(".agents/skills/tdd").display().to_string()
    );
    let agents = s.home.join(".agents/skills/tdd");
    assert!(agents.join("tests.md").is_file() && !agents.is_symlink());
    assert!(s.home.join(".pi/agent/skills/tdd/SKILL.md").is_file());
    assert!(!s.skills_dir().join("tdd").exists());
    // A copied skill leaves your local source when unmanaged.
    s.ok(&["skills", "unmanage", "my-notes"]);
    assert!(!s.data.join("local/skills/my-notes").exists());
    assert!(s.skills_dir().join("my-notes/SKILL.md").is_file());
    assert!(!s.skills_dir().join("my-notes").is_symlink());
    let inv = s.json(&["skills"]);
    assert_eq!(skill_named(&inv, "tdd")["state"], "unmanaged");
}

#[cfg(unix)]
#[test]
fn copies_that_differ_need_a_choice() {
    let (s, _) = world();
    // A second, different tdd in Claude Code's folder.
    installed(&s.skills_dir(), "tdd", "My own take on tdd.", None);
    let out = s.run(&["skills", "manage", "tdd", "--json"]);
    assert_eq!(out.code, 1);
    assert!(out.stdout.contains("copies differ"), "{}", out.stdout);
    let keep = s.home.join(".agents/skills/tdd");
    let r = s.json(&["skills", "manage", "tdd", "--path", keep.to_str().unwrap()]);
    assert_eq!(r["managed"][0]["originals"][0], keep.display().to_string());
    // The other copy is yours to sort out: it still blocks Claude Code.
    let inv = s.json(&["skills"]);
    let tdd = skill_named(&inv, "tdd");
    assert_eq!(tdd["state"], "partly-managed", "{tdd:#}");
    assert_eq!(tdd["issues"][0]["kind"], "blocked");
}

#[test]
fn a_plain_skills_repo_can_be_followed_directly() {
    let s = Sandbox::with_claude();
    let repo = plain_repo(&s);
    let r = s.json(&["init", &repo.url(), "--non-interactive"]);
    assert_eq!(r["kind"], "source", "{r:#}");
    let list = s.json(&["list"]);
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"acme-skills:skill/tdd"), "{ids:?}");
    assert!(s.skills_dir().join("grill-me/SKILL.md").is_file());
    let info = s.json(&["info", "acme-skills"]);
    assert!(
        info["description"]
            .as_str()
            .unwrap()
            .contains("no LOADOUT.md"),
        "{info:#}"
    );
    // Not editable through Loadout: there's nowhere to send changes.
    let out = s.run(&["export-source", "acme-skills", "--prepare"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("you follow"), "{}", out.stderr);
}

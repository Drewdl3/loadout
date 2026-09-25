//! Subagents (`agents/<name>.md`) and extras (`extras/<type>/<name>.md`)
//! install as single files.

mod common;

use common::Sandbox;

const REVIEWER: &str = "---\nname: reviewer\ndescription: Reviews diffs against Acme standards.\ntools: Read, Grep\n---\nYou are a careful reviewer.\n";
const DEPLOY: &str =
    "---\ndescription: Deploy the current branch to staging.\n---\nRun the deploy checklist.\n";

fn read(p: &std::path::Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

#[test]
fn agents_and_command_extras_install_as_files() {
    let s = Sandbox::with_claude();
    let agents = s.home.join(".claude/agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(agents.join("mine.md"), "my own agent").unwrap();
    std::fs::write(agents.join("planner.md"), "user's planner").unwrap();

    let repo = s.source("team-skills");
    repo.write("agents/reviewer.md", REVIEWER)
        .write("agents/planner.md", "---\ndescription: Theirs.\n---\n")
        .write("agents/README.md", "# docs, not an agent")
        .write("extras/commands/deploy.md", DEPLOY)
        .write(
            "extras/rules/style.md",
            "---\ndescription: Style rules.\n---\n",
        );
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    let out = s.ok(&["sync"]);
    assert!(
        out.stderr.contains("agents/planner.md already exists"),
        "collision warned\n{}",
        out.stderr
    );

    assert_eq!(read(&agents.join("reviewer.md")), REVIEWER);
    assert_eq!(read(&agents.join("planner.md")), "user's planner");
    assert_eq!(read(&agents.join("mine.md")), "my own agent");
    assert!(!agents.join("README.md").exists());
    assert_eq!(read(&s.home.join(".claude/commands/deploy.md")), DEPLOY);

    let list = s.json(&["list"]);
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "team-skills:agent/planner",
            "team-skills:agent/reviewer",
            "team-skills:extra/commands.deploy",
            "team-skills:extra/rules.style"
        ]
    );
    let why = s.json(&["why", "agent/reviewer"]);
    assert_eq!(why["winner"], "team-skills:agent/reviewer");

    // Upstream removes the agent: only our file goes.
    repo.remove("agents/reviewer.md");
    repo.commit("v2");
    s.ok(&["sync", "--yes"]);
    assert!(!agents.join("reviewer.md").exists());
    assert_eq!(read(&agents.join("mine.md")), "my own agent");
    assert_eq!(s.run(&["doctor"]).code, 0);
}

#[test]
fn invalid_extra_type_is_skipped_with_warning() {
    let s = Sandbox::with_claude();
    let repo = s.source("team-skills");
    repo.write("extras/Bad Type/x.md", "---\n---\n");
    repo.commit("v1");
    s.ok(&["subscribe", &repo.url()]);
    let out = s.ok(&["sync"]);
    assert!(out.stderr.contains("extras/Bad Type"), "{}", out.stderr);
}

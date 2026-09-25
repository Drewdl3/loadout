//! Project mode: a repo's `.loadout/` layer with its own
//! sources and committed lock, installed into project paths.

mod common;

use std::path::{Path, PathBuf};

use common::{Output, Sandbox, skill};
use loadout_git::fixture::FixtureRepo;

fn run_in(s: &Sandbox, dir: &Path, args: &[&str]) -> Output {
    let out = s.cmd().current_dir(dir).args(args).output().unwrap();
    Output {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn ok_in(s: &Sandbox, dir: &Path, args: &[&str]) -> Output {
    let out = run_in(s, dir, args);
    assert_eq!(out.code, 0, "{args:?}\n{}\n{}", out.stdout, out.stderr);
    out
}

fn code_repo(s: &Sandbox, name: &str) -> PathBuf {
    let dir = s.root.join("work").join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    s.git
        .run(Some(&dir), ["init", "--quiet", "--initial-branch", "main"])
        .unwrap();
    dir
}

fn app_skills(s: &Sandbox) -> FixtureRepo {
    let r = FixtureRepo::init(s.root.join("remotes/app-skills"), s.git.clone());
    r.write(
        "LOADOUT.md",
        "---\nloadout: 1\nname: app-skills\nlayer: team\ngroup: payments-dev\n---\n",
    )
    .write(
        "skills/deploy-app/SKILL.md",
        &skill("deploy-app", "Deploy this app."),
    )
    .write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "Project spec format."),
    )
    .write(
        "agents/app-expert.md",
        "---\ndescription: Knows this app.\n---\nYou know acme-app.\n",
    )
    .write(
        "mcp/app-db.md",
        "---\nname: app-db\ncommand: acme-db-mcp\nenv: { DB_PASSWORD: \"secret://app/db\" }\n---\n",
    );
    r.commit("v1");
    r
}

#[test]
fn project_layer_installs_into_the_repo_and_pins_in_it() {
    let s = Sandbox::with_claude();
    // User layer: a team source with its own write-spec.
    let team = s.source("team-skills");
    team.write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "Team format."),
    );
    team.commit("v1");
    s.ok(&["subscribe", &team.url()]);

    let app = app_skills(&s);
    let repo = code_repo(&s, "Acme App");
    let init = ok_in(&s, &repo.join("src"), &["init", "--project", "--json"]);
    let r: serde_json::Value = serde_json::from_str(&init.stdout).unwrap();
    assert_eq!(r["group"], "acme-app");
    assert!(repo.join(".loadout/config.toml").is_file());
    assert_ne!(
        run_in(&s, &repo, &["init", "--project"]).code,
        0,
        "only once"
    );

    ok_in(&s, &repo, &["--project", "subscribe", &app.url()]);
    let cfg = std::fs::read_to_string(repo.join(".loadout/config.toml")).unwrap();
    assert!(
        cfg.contains("layer = \"project\"") && cfg.contains("group = \"acme-app\""),
        "{cfg}"
    );

    // One sync inside the repo does both layers.
    let out = ok_in(&s, &repo.join("src"), &["sync", "--json"]);
    let r: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(r["project"]["sources"][0]["name"], "app-skills");

    let user_spec = std::fs::read_to_string(s.skills_dir().join("write-spec/SKILL.md")).unwrap();
    assert!(user_spec.contains("Team format."));
    let proj_spec =
        std::fs::read_to_string(repo.join(".claude/skills/write-spec/SKILL.md")).unwrap();
    assert!(proj_spec.contains("Project spec format."));
    assert!(repo.join(".claude/skills/deploy-app/SKILL.md").is_file());
    assert!(repo.join(".claude/agents/app-expert.md").is_file());
    assert!(
        !s.skills_dir().join("deploy-app").exists(),
        "project items stay in the project"
    );

    // Project MCP config; the secret wrapper knows which project it is.
    let mcp: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo.join(".mcp.json")).unwrap()).unwrap();
    let args: Vec<&str> = mcp["mcpServers"]["app-db"]["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(args[0], "--project-dir");
    assert_eq!(&args[2..], ["mcp-run", "app-skills:mcp/app-db"]);

    // The lock is in the repo; the user lock doesn't mention project sources.
    let plock = std::fs::read_to_string(repo.join(".loadout/loadout.lock")).unwrap();
    assert!(plock.contains("app-skills") && !plock.contains("layer"));
    let ulock = std::fs::read_to_string(s.data.join("loadout.lock")).unwrap();
    assert!(!ulock.contains("app-skills"));

    // Project items rank as `project`.
    let why = serde_json::from_str::<serde_json::Value>(
        &ok_in(
            &s,
            &repo,
            &["--project", "why", "skill/write-spec", "--json"],
        )
        .stdout,
    )
    .unwrap();
    assert_eq!(why["candidates"][0]["layer"], "project");
    assert_eq!(why["candidates"][0]["rank"], 35);

    // mcp-run finds the project's store from anywhere.
    let run = s.run(&[
        "--project-dir",
        repo.to_str().unwrap(),
        "mcp-run",
        "app-skills:mcp/app-db",
    ]);
    assert!(
        run.stderr.contains("secret://app/db: not found"),
        "{}",
        run.stderr
    );

    // Outside a project, --project explains itself.
    let out = s.run(&["--project", "list"]);
    assert_ne!(out.code, 0);
    assert!(
        out.stderr.contains("not inside a project"),
        "{}",
        out.stderr
    );
}

#[test]
fn teammate_gets_the_committed_pins() {
    let s = Sandbox::with_claude();
    let app = app_skills(&s);
    let repo = code_repo(&s, "acme-app");
    ok_in(&s, &repo, &["init", "--project"]);
    ok_in(&s, &repo, &["--project", "subscribe", &app.url()]);
    ok_in(&s, &repo, &["--project", "sync"]);

    // Upstream moves on after the lock was committed.
    app.write("skills/deploy-app/SKILL.md", &skill("deploy-app", "v2"));
    app.commit("v2");

    // A teammate clones the repo (with .loadout/) on another machine.
    let t = Sandbox::with_claude();
    let theirs = t.root.join("work/acme-app");
    std::fs::create_dir_all(theirs.join(".loadout")).unwrap();
    for f in ["config.toml", "loadout.lock"] {
        std::fs::copy(
            repo.join(".loadout").join(f),
            theirs.join(".loadout").join(f),
        )
        .unwrap();
    }
    ok_in(&t, &theirs, &["sync", "--locked"]);
    let deploy =
        std::fs::read_to_string(theirs.join(".claude/skills/deploy-app/SKILL.md")).unwrap();
    assert!(
        deploy.contains("Deploy this app."),
        "pinned version: {deploy}"
    );
}

//! Shortcodes: export on A, import on an offline B that has the
//! repos cached → identical fingerprint.

mod common;

use std::path::Path;

use common::{Sandbox, skill};
use loadout_git::fixture::FixtureRepo;

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), &dest).unwrap();
        }
    }
}

struct World {
    a: Sandbox,
    team: FixtureRepo,
    extra: FixtureRepo,
}

const TEAM: &[(&str, &str)] = &[("ACME_TEAM", "payments-dev")];

fn world() -> World {
    let a = Sandbox::with_claude();
    let team = FixtureRepo::init(a.root.join("remotes/payments-skills"), a.git.clone());
    team.write(
        "LOADOUT.md",
        "---\nloadout: 1\nname: payments-skills\nlayer: team\ngroup: payments-dev\n---\n",
    )
    .write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "Team v1."),
    )
    .write(
        "skills/runbook/SKILL.md",
        "---\ndescription: Off by default.\nloadout: { mode: default-off }\n---\n",
    );
    team.commit("v1");
    let company_config = FixtureRepo::init(a.root.join("remotes/acme-config"), a.git.clone());
    company_config
        .write(
            "LOADOUT.md",
            &format!(
                "---\nloadout: 1\nname: acme-config\nlayer: company\ngroup: acme\ncompany:\n  name: acme\n  layers:\n    - {{ name: company, rank: 0 }}\n    - {{ name: team, rank: 20 }}\n    - {{ name: role, rank: 40 }}\n  groups:\n    - layer: team\n      name: payments-dev\n      sources: [{:?}]\n      membership: {{ env: {{ ACME_TEAM: payments-dev }} }}\n---\n",
                team.url()
            ),
        )
        .write("skills/standards/SKILL.md", &skill("standards", "Company."));
    company_config.commit("company_config");
    let extra = FixtureRepo::init(a.root.join("remotes/extra-skills"), a.git.clone());
    extra
        .write(
            "LOADOUT.md",
            "---\nloadout: 1\nname: extra-skills\nlayer: role\ngroup: developer\n---\n",
        )
        .write("skills/extra/SKILL.md", &skill("extra", "Extra v1."));
    extra.commit("v1");

    let out = a.run_env(&["init", &company_config.url(), "--non-interactive"], TEAM);
    assert_eq!(out.code, 0, "{}", out.stderr);
    a.ok(&["subscribe", &extra.url(), "--priority", "3"]);
    assert_eq!(a.run_env(&["sync"], TEAM).code, 0);
    a.ok(&["enable", "payments-skills:skill/runbook"]);
    World { a, team, extra }
}

fn status_fp(s: &Sandbox) -> String {
    let st = s.run(&["status", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&st.stdout).unwrap();
    v["fingerprint"].as_str().unwrap().to_owned()
}

fn skills(s: &Sandbox) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(s.skills_dir()).unwrap() {
        let e = e.unwrap();
        let text = std::fs::read_to_string(e.path().join("SKILL.md")).unwrap();
        out.push((e.file_name().to_string_lossy().into_owned(), text));
    }
    out.sort();
    out
}

#[test]
fn export_on_a_import_on_offline_b_gives_identical_fingerprint() {
    let w = world();
    let fp_a = status_fp(&w.a);
    assert!(fp_a.starts_with("lo-fp:"), "{fp_a}");
    let export = w.a.json(&["export"]);
    let code = export["code"].as_str().unwrap().to_owned();
    assert_eq!(export["fingerprint"], fp_a.as_str());
    assert!(code.starts_with("lo1_"));

    // Upstream moves on after the export; the code pins A's commits.
    w.team.write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "Team v2."),
    );
    w.team.commit("v2");
    w.extra
        .write("skills/extra/SKILL.md", &skill("extra", "Extra v2."));
    w.extra.commit("v2");

    // B has A's clones cached, and no network: the remotes are gone.
    let b = Sandbox::with_claude();
    copy_dir(&w.a.data.join("repos"), &b.data.join("repos"));
    let remotes = w.a.root.join("remotes");
    let hidden = w.a.root.join("remotes-offline");
    std::fs::rename(&remotes, &hidden).unwrap();

    let out = b.run(&["import", &code, "--yes"]);
    std::fs::rename(&hidden, &remotes).unwrap();
    assert_eq!(out.code, 0, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(status_fp(&b), fp_a, "identical configuration");
    assert_eq!(skills(&b), skills(&w.a));
    let config = std::fs::read_to_string(b.config.join("config.toml")).unwrap();
    assert!(config.contains("priority = 3"), "{config}");
    assert!(config.contains("payments-skills:skill/runbook"), "{config}");
}

#[test]
fn import_needs_confirmation_and_rejects_damaged_codes() {
    let w = world();
    let code = w.a.json(&["export"])["code"].as_str().unwrap().to_owned();
    let b = Sandbox::with_claude();
    let out = b.run(&["import", &code]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("--yes"), "{}", out.stderr);
    assert!(!b.config.join("config.toml").exists(), "nothing written");

    // Change the last checksum character (to something it isn't already).
    let last = if code.ends_with('X') { 'Y' } else { 'X' };
    let damaged = format!("{}{last}", &code[..code.len() - 1]);
    let out = b.run(&["import", &damaged, "--yes"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("checksum"), "{}", out.stderr);
    assert!(
        b.run(&["import", "hello", "--yes"])
            .stderr
            .contains("not a Loadout shortcode")
    );
}

#[test]
fn latest_codes_have_no_pins_and_follow_upstream() {
    let w = world();
    let export = w.a.json(&["export", "--latest"]);
    assert_eq!(export["pinned"], false);
    let code = export["code"].as_str().unwrap().to_owned();
    w.extra
        .write("skills/extra/SKILL.md", &skill("extra", "Extra v2."));
    w.extra.commit("v2");
    let b = Sandbox::with_claude();
    let out = b.run(&["import", &code, "--yes"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let extra = std::fs::read_to_string(b.skills_dir().join("extra/SKILL.md")).unwrap();
    assert!(extra.contains("Extra v2."), "{extra}");
}

#[test]
fn qr_code_renders() {
    let w = world();
    let out = w.a.ok(&["export", "--qr"]);
    assert!(
        out.stdout.contains('█') || out.stdout.contains('▀'),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("lo1_"));
}

#[test]
fn dry_run_import_shows_the_changes_and_writes_nothing() {
    let w = world();
    let code = w.a.json(&["export"])["code"].as_str().unwrap().to_owned();
    let b = Sandbox::with_claude();
    let out = b.json(&["import", &code, "--dry-run"]);
    assert_eq!(out["applied"], false);
    let s = &out["summary"];
    assert_eq!(s["groups"][0], "company:acme");
    assert!(
        s["groups"]
            .as_array()
            .unwrap()
            .contains(&"team:payments-dev".into()),
        "{s}"
    );
    // Compared by name: Windows normalizes the fixture path's separators.
    assert!(
        s["sources"][0].as_str().unwrap().ends_with("extra-skills"),
        "{s}"
    );
    assert_eq!(s["toggles"]["payments-skills:skill/runbook"], true);
    assert!(!s["pins"].as_object().unwrap().is_empty(), "{s}");
    assert!(!b.config.join("config.toml").exists(), "nothing written");
    // --dry-run and --yes contradict each other.
    assert_eq!(b.run(&["import", &code, "--dry-run", "--yes"]).code, 2);
}

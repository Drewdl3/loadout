//! `lo search`.

mod common;

use common::{Sandbox, skill};
use loadout_git::fixture::FixtureRepo;

fn repo(s: &Sandbox, name: &str, layer: &str, group: &str, files: &[(&str, &str)]) -> FixtureRepo {
    let r = FixtureRepo::init(s.root.join("remotes").join(name), s.git.clone());
    r.write(
        "LOADOUT.md",
        &format!("---\nloadout: 1\nname: {name}\nlayer: {layer}\ngroup: {group}\n---\n"),
    );
    for (p, c) in files {
        r.write(p, c);
    }
    r.commit("init");
    r
}

fn world() -> Sandbox {
    let s = Sandbox::with_claude();
    let team = repo(
        &s,
        "payments-skills",
        "team",
        "payments-dev",
        &[
            (
                "skills/write-spec/SKILL.md",
                "---\ndescription: Write a feature spec in the Payments format.\nloadout: { tags: [specs, writing] }\n---\nSections: problem, goals, rollout.\n",
            ),
            (
                "skills/pci-checklist/SKILL.md",
                "---\ndescription: PCI checklist.\nloadout: { tags: [pci], applies_to: { role: [developer] } }\n---\nNo card data in logs.\n",
            ),
            (
                "mcp/jira.md",
                "---\ndescription: Jira tickets.\ncommand: jira-mcp\n---\n",
            ),
        ],
    );
    let pm = repo(
        &s,
        "pm-presets",
        "role",
        "product-manager",
        &[(
            "skills/prd-template/SKILL.md",
            &skill("prd-template", "Draft a PRD."),
        )],
    );
    let company_config = FixtureRepo::init(s.root.join("remotes/acme-config"), s.git.clone());
    company_config.write(
        "LOADOUT.md",
        &format!(
            "---\nloadout: 1\nname: acme-config\nlayer: company\ngroup: acme\ncompany:\n  name: acme\n  layers:\n    - {{ name: company, rank: 0 }}\n    - {{ name: team, rank: 20 }}\n    - {{ name: role, rank: 40 }}\n  groups:\n    - layer: team\n      name: payments-dev\n      sources: [{:?}]\n      membership: {{ manual: true }}\n    - layer: role\n      name: product-manager\n      sources: [{:?}]\n      membership: {{ manual: true }}\n---\n",
            team.url(),
            pm.url()
        ),
    );
    company_config.commit("company_config");
    s.ok(&["init", &company_config.url(), "--non-interactive"]);
    s.ok(&["join", "team:payments-dev"]);
    s
}

fn ids(v: &serde_json::Value) -> Vec<String> {
    v["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| {
            format!(
                "{} {}",
                h["id"].as_str().unwrap(),
                h["state"].as_str().unwrap()
            )
        })
        .collect()
}

#[test]
fn searches_subscribed_items() {
    let s = world();
    let r = s.json(&["search", "spec"]);
    assert_eq!(ids(&r)[0], "payments-skills:skill/write-spec enabled");
    let r = s.json(&["search", "card data"]);
    // Not a developer: the PCI checklist isn't for this profile.
    assert_eq!(ids(&r), ["payments-skills:skill/pci-checklist not-for-you"]);
    let r = s.json(&["search", "", "--kind", "mcp"]);
    assert_eq!(ids(&r), ["payments-skills:mcp/jira enabled"]);
    let r = s.json(&["search", "", "--tag", "writing"]);
    assert_eq!(ids(&r), ["payments-skills:skill/write-spec enabled"]);
    assert!(ids(&s.json(&["search", "prd"])).is_empty());

    let human = s.ok(&["search", "spec"]);
    s.insta()
        .bind(|| insta::assert_snapshot!("search_spec", human.stdout));
}

#[test]
fn all_sources_finds_groups_worth_joining() {
    let s = world();
    let r = s.json(&["search", "prd", "--all-sources"]);
    assert_eq!(ids(&r), ["pm-presets:skill/prd-template not-subscribed"]);
    assert_eq!(r["hits"][0]["layer"], "role");
    assert_eq!(r["hits"][0]["group"], "product-manager");
    // Discovery installs nothing.
    assert!(!s.skills_dir().join("prd-template").exists());
    s.ok(&["join", "role:product-manager"]);
    let r = s.json(&["search", "prd"]);
    assert_eq!(ids(&r), ["pm-presets:skill/prd-template enabled"]);
}

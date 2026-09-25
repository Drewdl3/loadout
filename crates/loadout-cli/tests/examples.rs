//! The `examples/` tree works end to end: each directory becomes a local
//! Git repo, the company config's `https://git.example.com/acme/<name>` URLs are
//! pointed at them, and `lo init` runs for a Payments developer.

mod common;

use common::{Sandbox, publish_examples};

const DEVELOPER: &[(&str, &str)] = &[
    ("ACME_ORG", "eng"),
    ("ACME_TEAM", "payments-dev"),
    ("ACME_ROLE", "developer"),
];

fn installed_skills(s: &Sandbox) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(s.skills_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn payments_developer_gets_the_right_setup() {
    let s = Sandbox::with_claude();
    let company_config = publish_examples(&s);
    let out = s.run_env(&["init", &company_config, "--non-interactive"], DEVELOPER);
    assert_eq!(out.code, 0, "{}\n{}", out.stdout, out.stderr);

    // Company (locked, plus the `lo` CLI skill), team override of the org's write-spec, and the
    // developer-only PCI checklist; not the PM preset or default-off items.
    assert_eq!(
        installed_skills(&s),
        [
            "code-review-standards",
            "loadout",
            "pci-checklist",
            "write-spec"
        ]
    );
    assert!(s.home.join(".claude/agents/code-reviewer.md").is_file());
    assert!(s.home.join(".claude/commands/reconcile.md").is_file());
    let spec = std::fs::read_to_string(s.skills_dir().join("write-spec/SKILL.md")).unwrap();
    assert!(spec.contains("Payments format"), "{spec}");

    let why = s.run_env(&["why", "skill/write-spec"], DEVELOPER);
    s.insta()
        .bind(|| insta::assert_snapshot!("why_write_spec", why.stdout));

    let list = s.run_env(&["list"], DEVELOPER);
    s.insta()
        .bind(|| insta::assert_snapshot!("list_developer", list.stdout));

    // MCP servers: Jira through the launch wrapper, docs through headersHelper.
    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(s.home.join(".claude.json")).unwrap())
            .unwrap();
    assert_eq!(cfg["mcpServers"]["jira"]["args"][0], "mcp-run");
    assert!(cfg["mcpServers"]["payments-docs"]["headersHelper"].is_string());

    // Opt into the Billing product; enable a default-off item.
    s.ok(&["join", "product:billing"]);
    s.ok(&["enable", "eng-skills:skill/incident-runbook"]);
    assert_eq!(
        installed_skills(&s),
        [
            "billing-glossary",
            "code-review-standards",
            "incident-runbook",
            "loadout",
            "pci-checklist",
            "write-spec"
        ]
    );

    // The company's locked, required item cannot be disabled.
    let out = s.run(&["disable", "acme-config:skill/code-review-standards"]);
    assert_ne!(out.code, 0);
}

#[test]
fn product_manager_gets_pm_presets_only() {
    let s = Sandbox::with_claude();
    let company_config = publish_examples(&s);
    let pm = &[("ACME_ROLE", "product-manager")];
    let out = s.run_env(&["init", &company_config, "--non-interactive"], pm);
    assert_eq!(out.code, 0, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(
        installed_skills(&s),
        ["code-review-standards", "loadout", "prd-template"]
    );
}

//! End-to-end: company config, membership discovery, `init`, `profile`,
//! `join` / `leave`. GitHub is a loopback mock; no external network.

mod common;

use common::{Output, Sandbox, skill};
use loadout_git::fixture::FixtureRepo;
use loadout_members::mock_http::MockServer;

fn repo(s: &Sandbox, name: &str, layer: &str, group: &str, skill_name: &str) -> FixtureRepo {
    let r = FixtureRepo::init(s.root.join("remotes").join(name), s.git.clone());
    r.write(
        "LOADOUT.md",
        &format!("---\nloadout: 1\nname: {name}\nlayer: {layer}\ngroup: {group}\n---\n"),
    )
    .write(
        &format!("skills/{skill_name}/SKILL.md"),
        &skill(skill_name, &format!("From {name}.")),
    );
    r.commit("init");
    r
}

/// Prints `["role:sre"]` on every OS.
fn exec_groups_command() -> String {
    if cfg!(windows) {
        r#"echo ["role:sre"]"#.to_owned()
    } else {
        r#"echo '["role:sre"]'"#.to_owned()
    }
}

struct World {
    s: Sandbox,
    company_config: FixtureRepo,
    github: MockServer,
}

impl World {
    fn new(allow_manual: bool) -> Self {
        let s = Sandbox::with_claude();
        let eng = repo(&s, "eng-skills", "org", "eng", "eng-tool");
        let cio = repo(&s, "cio-skills", "org", "cio", "cio-tool");
        let pay = repo(&s, "payments-skills", "team", "payments-dev", "pay-tool");
        let billing = repo(&s, "billing-config", "product", "billing", "billing-tool");
        let ci = repo(&s, "ci-presets", "role", "ci", "ci-tool");
        let sre = repo(&s, "sre-skills", "role", "sre", "sre-tool");

        let company_config = FixtureRepo::init(s.root.join("remotes/acme-config"), s.git.clone());
        let group = |layer: &str, name: &str, url: String, membership: &str| {
            format!(
                "    - layer: {layer}\n      name: {name}\n      sources: [{url:?}]\n      membership: {membership}\n"
            )
        };
        let mut groups = String::new();
        groups += &group(
            "org",
            "eng",
            eng.url(),
            r#"{ github_team: "acme/engineering" }"#,
        );
        groups += &group("org", "cio", cio.url(), r#"{ github_team: "acme/cio" }"#);
        groups += &group(
            "team",
            "payments-dev",
            pay.url(),
            r#"{ any: [ { github_team: "acme/payments-eng" }, { manual: true } ] }"#,
        );
        groups += &group("product", "billing", billing.url(), "{ manual: true }");
        groups += &group("role", "ci", ci.url(), r#"{ env: { LOADOUT_ROLE: "ci" } }"#);
        groups += &group(
            "role",
            "sre",
            sre.url(),
            &format!("{{ exec: {:?} }}", exec_groups_command()),
        );
        company_config
            .write(
                "LOADOUT.md",
                &format!(
                    "---\nloadout: 1\nname: acme-config\nlayer: company\ngroup: acme\ncompany:\n  name: acme\n  layers:\n    - {{ name: company, rank: 0 }}\n    - {{ name: org, rank: 10 }}\n    - {{ name: team, rank: 20 }}\n    - {{ name: product, rank: 25 }}\n    - {{ name: role, rank: 40 }}\n  groups:\n{groups}  policy:\n    allow_manual_sources: {allow_manual}\n---\n# Acme company config\n"
                ),
            )
            .write("skills/company-tool/SKILL.md", &skill("company-tool", "Company."));
        company_config.commit("company_config");

        // The user is in acme/engineering only.
        let github = MockServer::github("octo", &["acme/engineering"]);
        World {
            s,
            company_config,
            github,
        }
    }

    fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("GH_TOKEN", "test-token".to_owned()),
            ("LOADOUT_GITHUB_API", self.github.url().to_owned()),
        ]
    }

    fn run(&self, args: &[&str]) -> Output {
        let env = self.env();
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        self.s.run_env(args, &env)
    }

    fn ok(&self, args: &[&str]) -> Output {
        let out = self.run(args);
        assert_eq!(
            out.code, 0,
            "lo {args:?}\nstdout:\n{}\nstderr:\n{}",
            out.stdout, out.stderr
        );
        out
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        let mut a = args.to_vec();
        a.push("--json");
        serde_json::from_str(&self.ok(&a).stdout).unwrap()
    }

    fn installed(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.s.skills_dir())
            .map(|rd| {
                rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

fn members(v: &serde_json::Value) -> Vec<String> {
    v["groups"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|u| u["member"] == true)
        .map(|u| {
            format!(
                "{}:{}",
                u["layer"].as_str().unwrap(),
                u["group"].as_str().unwrap()
            )
        })
        .collect()
}

// `lo init <company-config>` on a fresh machine yields the
// correct groups from GitHub teams with no other input.
#[test]
fn init_discovers_groups_with_no_other_input() {
    let w = World::new(true);
    let v = w.json(&["init", &w.company_config.url(), "--non-interactive"]);
    assert_eq!(members(&v), ["company:acme", "org:eng", "role:sre"]);
    assert_eq!(v["targets"], serde_json::json!(["claude-code"]));
    assert_eq!(w.installed(), ["company-tool", "eng-tool", "sre-tool"]);

    // The GitHub API was asked with the user's token.
    let reqs = w.github.requests();
    assert!(
        reqs.iter()
            .any(|r| r.path == "/orgs/acme/teams/engineering/memberships/octo")
    );
    assert!(
        reqs.iter()
            .all(|r| r.authorization.as_deref() == Some("Bearer test-token"))
    );

    let config = std::fs::read_to_string(w.s.config.join("config.toml")).unwrap();
    assert!(config.contains("company_config = "), "{config}");
    assert!(config.contains("company = \"acme\""), "{config}");
    assert!(config.contains("org = [\"eng\"]"), "{config}");

    let out = w.ok(&["profile"]);
    w.s.insta()
        .bind(|| insta::assert_snapshot!("profile_show", out.stdout));
}

#[test]
fn env_membership_for_ci() {
    let w = World::new(true);
    let mut env = w.env();
    env.push(("LOADOUT_ROLE", "ci".to_owned()));
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let out = w.s.run_env(
        &[
            "init",
            &w.company_config.url(),
            "--non-interactive",
            "--json",
        ],
        &env,
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert!(members(&v).contains(&"role:ci".to_owned()));
    assert!(w.installed().contains(&"ci-tool".to_owned()));
}

#[test]
fn join_and_leave_manual_groups() {
    let w = World::new(true);
    w.ok(&["init", &w.company_config.url(), "--non-interactive"]);

    w.ok(&["join", "product:billing"]);
    assert!(w.installed().contains(&"billing-tool".to_owned()));
    // any: [github_team, manual] is joinable too.
    w.ok(&["join", "team:payments-dev"]);
    assert!(w.installed().contains(&"pay-tool".to_owned()));

    let out = w.run(&["join", "org:cio"]);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr
            .contains("decided by the company config (github_team)"),
        "{}",
        out.stderr
    );
    assert_eq!(w.run(&["join", "org:nope"]).code, 1);
    assert_eq!(w.run(&["join", "company:acme"]).code, 1);

    w.ok(&["leave", "product:billing"]);
    assert!(!w.installed().contains(&"billing-tool".to_owned()));
    // Leaving a discovered group sticks across refreshes.
    w.ok(&["leave", "org:eng"]);
    assert!(!w.installed().contains(&"eng-tool".to_owned()));
    let v = w.json(&["profile", "refresh"]);
    assert!(!members(&v["profile"]).contains(&"org:eng".to_owned()));

    let config = std::fs::read_to_string(w.s.config.join("config.toml")).unwrap();
    assert!(config.contains("[membership]"), "{config}");
}

#[test]
fn profile_set_overrides_one_layer() {
    let w = World::new(true);
    w.ok(&["init", &w.company_config.url(), "--non-interactive"]);
    let v = w.json(&["profile", "set", "org", "cio"]);
    let m = members(&v["profile"]);
    assert!(m.contains(&"org:cio".to_owned()));
    assert!(!m.contains(&"org:eng".to_owned()));
    assert!(w.installed().contains(&"cio-tool".to_owned()));
    assert!(!w.installed().contains(&"eng-tool".to_owned()));
    assert_eq!(w.run(&["profile", "set", "org", "nope"]).code, 1);
}

#[test]
fn provider_failure_keeps_cached_membership() {
    let w = World::new(true);
    w.ok(&["init", &w.company_config.url(), "--non-interactive"]);
    // Force a refresh on the next sync with a token GitHub rejects.
    let bad = MockServer::start(
        [("/user".to_owned(), (401, "{}".to_owned()))]
            .into_iter()
            .collect(),
    );
    let state = w.s.data.join("membership.json");
    let text = std::fs::read_to_string(&state).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
    v["refreshed_at"] = 0.into();
    std::fs::write(&state, v.to_string()).unwrap();

    let out = w.s.run_env(
        &["sync", "--json"],
        &[("GH_TOKEN", "bad"), ("LOADOUT_GITHUB_API", bad.url())],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["profile_refreshed"], true);
    let warnings = v["warnings"].to_string();
    assert!(
        warnings.contains("keeping cached value (member)"),
        "{warnings}"
    );
    assert!(
        !warnings.contains("bad\""),
        "token must not leak: {warnings}"
    );
    assert!(w.installed().contains(&"eng-tool".to_owned()));
}

#[test]
fn sync_refreshes_profile_only_when_due() {
    let w = World::new(true);
    w.ok(&["init", &w.company_config.url(), "--non-interactive"]);
    let before = w.github.requests().len();
    let v = w.json(&["sync"]);
    assert_eq!(v["profile_refreshed"], false);
    assert_eq!(w.github.requests().len(), before, "no discovery within 24h");
}

#[test]
fn manual_sources_follow_company_config_policy() {
    let w = World::new(false);
    w.ok(&["init", &w.company_config.url(), "--non-interactive"]);
    let extra = repo(&w.s, "extra", "team", "x", "extra-tool");
    let out = w.run(&["subscribe", &extra.url()]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("does not allow"), "{}", out.stderr);

    // Subscribing to a company config-listed source is fine.
    let listed = w.s.root.join("remotes/billing-config");
    w.ok(&["subscribe", &listed.to_string_lossy()]);
}

#[test]
fn init_refuses_a_different_company_config() {
    let w = World::new(true);
    w.ok(&["init", &w.company_config.url(), "--non-interactive"]);
    w.ok(&["init", &w.company_config.url(), "--non-interactive"]);
    let other = repo(&w.s, "not-a-config", "team", "x", "t");
    let out = w.run(&["init", &other.url(), "--non-interactive"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("already set up"), "{}", out.stderr);
}

#[test]
fn init_rejects_non_company_config_repo() {
    let s = Sandbox::with_claude();
    let plain = repo(&s, "plain", "team", "x", "t");
    let out = s.run(&["init", &plain.url(), "--non-interactive"]);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("not a company config"),
        "{}",
        out.stderr
    );
}

//! Loadout membership discovery.
//!
//! [`discover`] evaluates every company config group's membership rule through
//! pluggable [`Providers`], combines the results with the user's manual
//! `joined`/`left` choices, and falls back to the cached profile for groups
//! whose providers fail (no token, offline, …).

use std::collections::BTreeMap;

use loadout_model::{CompanyConfig, GroupDef, Membership, Profile, Rule};
use serde::{Deserialize, Serialize};

pub mod exec;
pub mod github;
pub mod gitlab;
#[cfg(feature = "test-support")]
pub mod mock_http;
pub mod repo;

/// A provider could not decide (as opposed to deciding "not a member").
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{provider}: {message}")]
pub struct ProviderError {
    pub provider: &'static str,
    pub message: String,
}

impl ProviderError {
    pub fn new(provider: &'static str, message: impl Into<String>) -> Self {
        ProviderError {
            provider,
            message: message.into(),
        }
    }
}

/// `github_team` membership.
pub trait GithubTeams {
    /// Whether the current user is an active member of `org/team`.
    fn is_member(&self, org: &str, team: &str) -> Result<bool, ProviderError>;
}

/// `gitlab_group` membership.
pub trait GitlabGroups {
    /// Whether the current user is a member of the group (directly or
    /// through a parent group).
    fn is_member(&self, group: &str) -> Result<bool, ProviderError>;
}

/// `exec` membership: runs a company command returning a JSON group list.
pub trait GroupCommand {
    fn groups(&self, command: &str) -> Result<Vec<String>, ProviderError>;
}

/// `repo_access` membership.
pub trait RepoAccess {
    fn can_access(&self, url: &str) -> Result<bool, ProviderError>;
}

/// Everything rule evaluation may consult. All I/O sits behind these.
pub struct Providers<'a> {
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub github: &'a dyn GithubTeams,
    pub gitlab: &'a dyn GitlabGroups,
    pub exec: &'a dyn GroupCommand,
    pub repo: &'a dyn RepoAccess,
}

/// Evaluates `rule` for `group`. `manual: true` alone never makes someone a
/// member; manual membership comes from `lo join`.
pub fn evaluate(rule: &Rule, group: &GroupDef, p: &Providers<'_>) -> Result<bool, ProviderError> {
    match rule {
        Rule::Manual(_) => Ok(false),
        Rule::Env(vars) => Ok(vars
            .iter()
            .all(|(k, v)| (p.env)(k).is_some_and(|actual| &actual == v))),
        Rule::GithubTeam(spec) => {
            let (org, team) = spec.split_once('/').ok_or_else(|| {
                ProviderError::new(
                    "github_team",
                    format!("expected \"org/team\", got {spec:?}"),
                )
            })?;
            p.github.is_member(org, team)
        }
        Rule::GitlabGroup(group) => {
            if group.trim().is_empty() {
                return Err(ProviderError::new("gitlab_group", "empty group path"));
            }
            p.gitlab.is_member(group.trim_matches('/'))
        }
        Rule::Exec(command) => {
            let groups = p.exec.groups(command)?;
            let id = group.id();
            Ok(groups.iter().any(|g| *g == group.name || *g == id))
        }
        Rule::RepoAccess(false) => Ok(false),
        Rule::RepoAccess(true) => match group.sources.first() {
            Some(url) => p.repo.can_access(url),
            None => Err(ProviderError::new("repo_access", "group lists no sources")),
        },
        Rule::Any(rules) => {
            let mut first_err = None;
            for r in rules {
                match evaluate(r, group, p) {
                    Ok(true) => return Ok(true),
                    Ok(false) => {}
                    Err(e) => {
                        first_err.get_or_insert(e);
                    }
                }
            }
            first_err.map_or(Ok(false), Err)
        }
        Rule::All(rules) => {
            let mut first_err = None;
            for r in rules {
                match evaluate(r, group, p) {
                    Ok(true) => {}
                    Ok(false) => return Ok(false),
                    Err(e) => {
                        first_err.get_or_insert(e);
                    }
                }
            }
            first_err.map_or(Ok(!rules.is_empty()), Err)
        }
    }
}

/// How a group's membership was decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// The company's own group: everyone.
    Company,
    /// The membership rule was evaluated.
    Rule,
    /// `lo join`.
    Joined,
    /// `lo leave`.
    Left,
    /// A provider failed; the cached profile was used.
    Cached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GroupStatus {
    pub layer: String,
    pub group: String,
    pub member: bool,
    pub basis: Basis,
    /// Whether `lo join` is allowed for this group.
    pub joinable: bool,
    /// Provider error when `basis` is `cached`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    pub profile: Profile,
    pub groups: Vec<GroupStatus>,
    pub warnings: Vec<String>,
}

/// Computes the profile from the company config.
pub fn discover(
    company_config: &CompanyConfig,
    membership: &Membership,
    cached: &Profile,
    providers: &Providers<'_>,
) -> Discovery {
    let mut out = Discovery::default();
    out.profile.add("company", company_config.name.as_str());
    out.groups.push(GroupStatus {
        layer: "company".into(),
        group: company_config.name.clone(),
        member: true,
        basis: Basis::Company,
        joinable: false,
        error: None,
    });
    for group in &company_config.groups {
        let id = group.id();
        let (member, basis, error) = if membership.has_left(&id) {
            (false, Basis::Left, None)
        } else if membership.has_joined(&id) {
            (true, Basis::Joined, None)
        } else {
            match evaluate(&group.membership, group, providers) {
                Ok(m) => (m, Basis::Rule, None),
                Err(e) => {
                    let was = cached.contains(&group.layer, &group.name);
                    out.warnings.push(format!(
                        "could not check membership of {id} ({e}); keeping cached value ({})",
                        if was { "member" } else { "not a member" }
                    ));
                    (was, Basis::Cached, Some(e.to_string()))
                }
            }
        };
        if member {
            out.profile.add(group.layer.as_str(), group.name.as_str());
        }
        out.groups.push(GroupStatus {
            layer: group.layer.clone(),
            group: group.name.clone(),
            member,
            basis,
            joinable: group.membership.allows_manual(),
            error,
        });
    }
    out
}

/// A profile as the `[profile]` table: layer → groups.
pub fn profile_table(p: &Profile) -> BTreeMap<String, Vec<String>> {
    let mut t: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (layer, group) in p.iter() {
        t.entry(layer.to_owned())
            .or_default()
            .push(group.to_owned());
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeSet;

    struct FakeGithub {
        teams: BTreeSet<&'static str>,
        fail: bool,
        calls: RefCell<Vec<String>>,
    }
    impl GithubTeams for FakeGithub {
        fn is_member(&self, org: &str, team: &str) -> Result<bool, ProviderError> {
            self.calls.borrow_mut().push(format!("{org}/{team}"));
            if self.fail {
                return Err(ProviderError::new("github_team", "no token"));
            }
            Ok(self.teams.contains(format!("{org}/{team}").as_str()))
        }
    }
    struct FakeExec(Result<Vec<String>, ProviderError>);
    impl GroupCommand for FakeExec {
        fn groups(&self, _: &str) -> Result<Vec<String>, ProviderError> {
            self.0.clone()
        }
    }
    struct FakeRepo(bool);
    impl RepoAccess for FakeRepo {
        fn can_access(&self, _: &str) -> Result<bool, ProviderError> {
            Ok(self.0)
        }
    }

    fn company_config() -> CompanyConfig {
        let yaml = r#"
name: acme
layers: [{name: company, rank: 0}, {name: org, rank: 10}, {name: team, rank: 20}, {name: product, rank: 25}, {name: role, rank: 40}]
groups:
  - { layer: org, name: eng, sources: [x], membership: { github_team: "acme/engineering" } }
  - { layer: org, name: cio, sources: [x], membership: { github_team: "acme/cio" } }
  - { layer: team, name: payments-dev, sources: [x], membership: { any: [ { github_team: "acme/payments-eng" }, { manual: true } ] } }
  - { layer: product, name: billing, sources: [x], membership: { manual: true } }
  - { layer: role, name: ci, sources: [x], membership: { env: { LOADOUT_ROLE: ci } } }
  - { layer: role, name: sre, sources: [x], membership: { exec: "groups --json" } }
  - { layer: team, name: private, sources: [x], membership: { repo_access: true } }
  - { layer: team, name: both, sources: [x], membership: { all: [ { github_team: "acme/engineering" }, { env: { ONCALL: "1" } } ] } }
"#;
        let v: serde_json::Value = serde_saphyr::from_str(yaml).unwrap();
        CompanyConfig::from_value(&v).unwrap()
    }

    fn run(
        github: &FakeGithub,
        env: &[(&str, &str)],
        membership: &Membership,
        cached: &Profile,
    ) -> Discovery {
        let env: BTreeMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let env_fn = move |k: &str| env.get(k).cloned();
        let exec = FakeExec(Ok(vec!["role:sre".into(), "unrelated".into()]));
        let repo = FakeRepo(true);
        discover(
            &company_config(),
            membership,
            cached,
            &Providers {
                env: &env_fn,
                github,
                gitlab: &FakeGitlab,
                exec: &exec,
                repo: &repo,
            },
        )
    }

    /// Member of `acme/payments` only.
    struct FakeGitlab;

    impl GitlabGroups for FakeGitlab {
        fn is_member(&self, group: &str) -> Result<bool, ProviderError> {
            Ok(group == "acme/payments")
        }
    }

    #[test]
    fn gitlab_group_rule() {
        let group = |g: &str| -> GroupDef {
            serde_json::from_value(serde_json::json!({
                "layer": "team", "name": "x", "membership": { "gitlab_group": g }
            }))
            .unwrap()
        };
        let gh = github(&[]);
        let env = |_: &str| None;
        let p = Providers {
            env: &env,
            github: &gh,
            gitlab: &FakeGitlab,
            exec: &FakeExec(Ok(vec![])),
            repo: &FakeRepo(false),
        };
        let yes = group("/acme/payments/");
        assert_eq!(evaluate(&yes.membership, &yes, &p), Ok(true));
        let no = group("acme/other");
        assert_eq!(evaluate(&no.membership, &no, &p), Ok(false));
        let empty = group(" ");
        assert!(evaluate(&empty.membership, &empty, &p).is_err());
    }

    fn github(teams: &[&'static str]) -> FakeGithub {
        FakeGithub {
            teams: teams.iter().copied().collect(),
            fail: false,
            calls: RefCell::default(),
        }
    }

    fn members(d: &Discovery) -> Vec<String> {
        d.profile.iter().map(|(s, u)| format!("{s}:{u}")).collect()
    }

    #[test]
    fn discovers_groups_from_providers() {
        let d = run(
            &github(&["acme/engineering", "acme/payments-eng"]),
            &[],
            &Membership::default(),
            &Profile::default(),
        );
        assert_eq!(
            members(&d),
            [
                "company:acme",
                "org:eng",
                "role:sre",
                "team:payments-dev",
                "team:private"
            ]
        );
        assert!(d.warnings.is_empty());
        let billing = d.groups.iter().find(|u| u.group == "billing").unwrap();
        assert!(!billing.member && billing.joinable);
    }

    #[test]
    fn env_all_and_manual_choices() {
        let d = run(
            &github(&["acme/engineering"]),
            &[("LOADOUT_ROLE", "ci"), ("ONCALL", "1")],
            &Membership {
                joined: vec!["product:billing".into()],
                left: vec!["org:eng".into()],
            },
            &Profile::default(),
        );
        let m = members(&d);
        assert!(m.contains(&"role:ci".to_owned()));
        assert!(m.contains(&"team:both".to_owned()));
        assert!(m.contains(&"product:billing".to_owned()));
        assert!(
            !m.contains(&"org:eng".to_owned()),
            "left wins over discovery"
        );
        let eng = d.groups.iter().find(|u| u.group == "eng").unwrap();
        assert_eq!(eng.basis, Basis::Left);
    }

    #[test]
    fn provider_failure_falls_back_to_cache() {
        let failing = FakeGithub {
            teams: BTreeSet::new(),
            fail: true,
            calls: RefCell::default(),
        };
        let cached: Profile = [("org", "eng")].into_iter().collect();
        let d = run(&failing, &[], &Membership::default(), &cached);
        let m = members(&d);
        assert!(m.contains(&"org:eng".to_owned()), "cached membership kept");
        assert!(!m.contains(&"org:cio".to_owned()));
        let eng = d.groups.iter().find(|u| u.group == "eng").unwrap();
        assert_eq!(eng.basis, Basis::Cached);
        assert!(
            d.warnings
                .iter()
                .any(|w| w.contains("org:eng") && w.contains("no token"))
        );
        // any: [github (fails), manual] with no join → falls back to cache too.
        let pay = d.groups.iter().find(|u| u.group == "payments-dev").unwrap();
        assert_eq!(pay.basis, Basis::Cached);
    }

    #[test]
    fn any_short_circuits_on_success() {
        let gh = github(&["acme/payments-eng"]);
        let d = run(&gh, &[], &Membership::default(), &Profile::default());
        assert!(members(&d).contains(&"team:payments-dev".to_owned()));
    }

    #[test]
    fn malformed_github_team_is_an_error() {
        let group: GroupDef = serde_json::from_value(serde_json::json!({
            "layer": "team", "name": "x", "membership": { "github_team": "no-slash" }
        }))
        .unwrap();
        let gh = github(&[]);
        let env = |_: &str| None;
        let p = Providers {
            env: &env,
            github: &gh,
            gitlab: &FakeGitlab,
            exec: &FakeExec(Ok(vec![])),
            repo: &FakeRepo(false),
        };
        assert!(evaluate(&group.membership, &group, &p).is_err());
        assert!(gh.calls.borrow().is_empty());
    }
}

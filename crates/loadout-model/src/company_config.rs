//! The company config: the `company:` block of the company's root `LOADOUT.md`
//!.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ModelError;
use crate::config::normalize_url;
use crate::layer::{Layer, LayerModel};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompanyConfig {
    /// The company's name: its group everyone is in.
    pub name: String,
    /// Layer ranks; higher is more specific.
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub groups: Vec<GroupDef>,
    #[serde(default)]
    pub policy: Policy,
    /// Secret settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<CompanySecrets>,
    /// Allowed commit signers for `policy.require_signed` layers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signers: Vec<Signer>,
    /// Extra audit rules path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_rules: Option<String>,
}

/// An allowed commit signer: an SSH public key or an armored GPG
/// public key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Signer {
    /// Who this is (shown when a signature verifies), e.g. an email.
    pub principal: String,
    /// `ssh-ed25519 AAAA…` (as in an `allowed_signers` file, without the
    /// principal).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<String>,
    /// An ASCII-armored OpenPGP public key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpg: Option<String>,
}

/// `company.secrets`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CompanySecrets {
    /// Resolver chain for `secret://name`: `env`, `keychain`, or reference
    /// templates containing `{name}`.
    pub chain: Vec<String>,
}

/// A named instance of a layer and the sources it publishes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupDef {
    pub layer: String,
    pub name: String,
    #[serde(default)]
    pub sources: Vec<String>,
    /// How membership is discovered. Omitted means `{ manual: true }`.
    #[serde(default = "Rule::manual")]
    pub membership: Rule,
}

impl GroupDef {
    /// `layer:name`.
    pub fn id(&self) -> String {
        format!("{}:{}", self.layer, self.name)
    }
}

/// A membership rule. Written as a single-key map, e.g.
/// `{ github_team: "acme/eng" }` or `{ any: [ … ] }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    Manual(bool),
    GithubTeam(String),
    GitlabGroup(String),
    Env(BTreeMap<String, String>),
    Exec(String),
    RepoAccess(bool),
    Any(Vec<Rule>),
    All(Vec<Rule>),
}

impl Rule {
    pub fn manual() -> Rule {
        Rule::Manual(true)
    }

    /// Whether a user may opt in by hand (`lo join`).
    pub fn allows_manual(&self) -> bool {
        match self {
            Rule::Manual(on) => *on,
            Rule::Any(rules) => rules.iter().any(Rule::allows_manual),
            Rule::All(rules) => !rules.is_empty() && rules.iter().all(Rule::allows_manual),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    /// Layers whose updates apply without review.
    pub auto_apply: Vec<String>,
    /// Layers whose commits must be signed.
    pub require_signed: Vec<String>,
    /// May users subscribe to repos not listed in the company config?
    pub allow_manual_sources: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            auto_apply: Vec::new(),
            require_signed: Vec::new(),
            allow_manual_sources: true,
        }
    }
}

impl CompanyConfig {
    /// Parses and validates a `company:` block (already extracted from
    /// `LOADOUT.md` frontmatter).
    pub fn from_value(value: &Value) -> Result<Self, ModelError> {
        let company_config: CompanyConfig = serde_json::from_value(value.clone())
            .map_err(|e| ModelError::CompanyConfig(e.to_string()))?;
        company_config.validate()?;
        Ok(company_config)
    }

    fn validate(&self) -> Result<(), ModelError> {
        let err = |m: String| Err(ModelError::CompanyConfig(m));
        if self.name.trim().is_empty() {
            return err("company must not be empty".into());
        }
        if self.layers.is_empty() {
            return err("layers must list at least one layer".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for s in &self.layers {
            if !seen.insert(s.name.as_str()) {
                return err(format!("layer {:?} is listed twice", s.name));
            }
        }
        if !seen.contains("company") {
            return err("layers must include `company`".into());
        }
        let mut groups = std::collections::BTreeSet::new();
        for u in &self.groups {
            if !seen.contains(u.layer.as_str()) {
                return err(format!(
                    "group {} uses undefined layer {:?}",
                    u.name, u.layer
                ));
            }
            if !groups.insert(u.id()) {
                return err(format!("group {} is listed twice", u.id()));
            }
        }
        for signer in &self.signers {
            let p = &signer.principal;
            if p.trim().is_empty() || p.chars().any(char::is_whitespace) {
                return err(format!(
                    "signer principal {p:?} must be non-empty without spaces"
                ));
            }
            match (&signer.ssh, &signer.gpg) {
                (Some(k), None) => {
                    let parts: Vec<&str> = k.split_whitespace().collect();
                    if parts.len() < 2
                        || !(parts[0].starts_with("ssh-")
                            || parts[0].starts_with("ecdsa-")
                            || parts[0].starts_with("sk-"))
                    {
                        return err(format!(
                            "signer {p}: `ssh` must be a public key like \"ssh-ed25519 AAAA…\""
                        ));
                    }
                }
                (None, Some(k)) => {
                    if !k.contains("BEGIN PGP PUBLIC KEY BLOCK") {
                        return err(format!(
                            "signer {p}: `gpg` must be an ASCII-armored public key"
                        ));
                    }
                }
                _ => return err(format!("signer {p}: set exactly one of `ssh` or `gpg`")),
            }
        }
        for s in self
            .policy
            .auto_apply
            .iter()
            .chain(&self.policy.require_signed)
        {
            if !seen.contains(s.as_str()) {
                return err(format!("policy names undefined layer {s:?}"));
            }
        }
        Ok(())
    }

    /// The company config's layers, plus `user` above them all unless the
    /// company config ranks it itself.
    pub fn layer_model(&self) -> LayerModel {
        LayerModel::new(self.layers.iter().map(|s| (s.name.as_str(), s.rank))).with_user_layer()
    }

    pub fn group(&self, layer: &str, name: &str) -> Option<&GroupDef> {
        self.groups
            .iter()
            .find(|u| u.layer == layer && u.name == name)
    }

    /// Whether `url` is one of the company config's group sources.
    pub fn lists_source(&self, url: &str) -> bool {
        let url = normalize_url(url);
        self.groups
            .iter()
            .flat_map(|u| &u.sources)
            .any(|s| normalize_url(s) == url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ManifestDoc;

    pub const EXAMPLE_COMPANY: &str = r#"---
loadout: 1
name: acme-config
layer: company
group: acme
company:
  name: acme
  layers:
    - { name: company, rank: 0 }
    - { name: org,     rank: 10 }
    - { name: team,    rank: 20 }
    - { name: product, rank: 25 }
    - { name: squad,   rank: 30 }
    - { name: role,    rank: 40 }
  groups:
    - layer: org
      name: cio
      sources: ["https://example.com/acme-cio/agent-skills"]
      membership: { github_team: "acme/cio" }
    - layer: org
      name: eng
      sources: ["https://example.com/acme/eng-skills"]
      membership: { github_team: "acme/engineering" }
    - layer: team
      name: payments-dev
      sources: ["https://example.com/acme/payments-skills"]
      membership: { any: [ { github_team: "acme/payments-eng" }, { manual: true } ] }
    - layer: product
      name: billing
      sources: ["https://example.com/acme/billing-agent-config"]
      membership: { manual: true }
    - layer: role
      name: ci
      sources: ["https://example.com/acme/ci-agent-presets"]
      membership: { env: { LOADOUT_ROLE: "ci" } }
  policy:
    auto_apply: [company]
    require_signed: [company]
    allow_manual_sources: true
---
"#;

    fn parse(text: &str) -> Result<CompanyConfig, ModelError> {
        let doc = ManifestDoc::parse(text).unwrap();
        CompanyConfig::from_value(doc.manifest.company_config.as_ref().unwrap())
    }

    #[test]
    fn parses_example() {
        let c = parse(EXAMPLE_COMPANY).unwrap();
        assert_eq!(c.name, "acme");
        assert_eq!(c.layer_model().rank("role"), Some(40));
        assert_eq!(c.groups.len(), 5);
        let pay = c.group("team", "payments-dev").unwrap();
        assert_eq!(
            pay.membership,
            Rule::Any(vec![
                Rule::GithubTeam("acme/payments-eng".into()),
                Rule::Manual(true)
            ])
        );
        assert!(pay.membership.allows_manual());
        assert!(!c.group("org", "eng").unwrap().membership.allows_manual());
        let ci = c.group("role", "ci").unwrap();
        assert_eq!(
            ci.membership,
            Rule::Env([("LOADOUT_ROLE".to_owned(), "ci".to_owned())].into())
        );
        assert_eq!(c.policy.auto_apply, ["company"]);
        assert!(c.lists_source("https://example.com/acme/eng-skills.git"));
        assert!(!c.lists_source("https://example.com/someone/else"));
    }

    #[test]
    fn membership_defaults_to_manual() {
        let c = parse(
            "---\nloadout: 1\nname: c\nlayer: company\ncompany:\n  name: acme\n  layers: [{ name: company, rank: 0 }, { name: team, rank: 20 }]\n  groups: [{ layer: team, name: t }]\n---\n",
        )
        .unwrap();
        assert_eq!(c.groups[0].membership, Rule::Manual(true));
        assert!(c.policy.allow_manual_sources);
    }

    #[test]
    fn rejects_invalid_company_configs() {
        let base =
            |body: &str| format!("---\nloadout: 1\nname: c\nlayer: company\ncompany:\n{body}---\n");
        let cases = [
            ("no layers", "  name: acme\n  layers: []\n"),
            (
                "no company layer",
                "  name: acme\n  layers: [{ name: team, rank: 1 }]\n",
            ),
            (
                "duplicate layer",
                "  name: acme\n  layers: [{ name: company, rank: 0 }, { name: company, rank: 1 }]\n",
            ),
            (
                "undefined group layer",
                "  name: acme\n  layers: [{ name: company, rank: 0 }]\n  groups: [{ layer: team, name: t }]\n",
            ),
            (
                "unknown rule",
                "  name: acme\n  layers: [{ name: company, rank: 0 }, { name: team, rank: 1 }]\n  groups: [{ layer: team, name: t, membership: { okta: x } }]\n",
            ),
            (
                "policy layer",
                "  name: acme\n  layers: [{ name: company, rank: 0 }]\n  policy: { auto_apply: [team] }\n",
            ),
            (
                "unknown key",
                "  name: acme\n  layers: [{ name: company, rank: 0 }]\n  extra: 1\n",
            ),
        ];
        for (what, body) in cases {
            assert!(parse(&base(body)).is_err(), "{what} should be rejected");
        }
    }

    #[test]
    fn signers_are_validated() {
        let with = |signers: &str| {
            parse(
                &EXAMPLE_COMPANY
                    .replace("  policy:\n", &format!("  signers:\n{signers}  policy:\n")),
            )
        };
        let c =
            with("    - { principal: release@acme.example.com, ssh: \"ssh-ed25519 AAAAC3Nza\" }\n")
                .unwrap();
        assert_eq!(c.signers[0].principal, "release@acme.example.com");
        assert!(with("    - { principal: x, ssh: \"not a key\" }\n").is_err());
        assert!(with("    - { principal: x }\n").is_err());
        assert!(with("    - { principal: \"a b\", ssh: \"ssh-ed25519 AAAA\" }\n").is_err());
        assert!(
            with("    - { principal: x, gpg: \"-----BEGIN PGP PUBLIC KEY BLOCK-----\" }\n").is_ok()
        );
    }
}

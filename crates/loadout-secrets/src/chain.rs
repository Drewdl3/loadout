//! The resolver chain for `secret://<name>`: configured in the
//! company config (`company.secrets.chain`) and overridable locally.
//!
//! Entries are tried in order until one finds a value:
//!
//! | Entry | Looks up `secret://jira/token` as |
//! |---|---|
//! | `env` | environment variable `JIRA_TOKEN` |
//! | `keychain` | keychain service `loadout`, account `jira/token` |
//! | a URI template with `{name}`, e.g. `op://Acme/{name}` | `op://Acme/jira/token` |

use std::fmt;
use std::str::FromStr;

use loadout_model::SecretRef;
use secrecy::SecretString;

use crate::keystore::KEYCHAIN_SERVICE;
use crate::{Resolvers, SecretError};

/// The chain used when neither the company config nor the local config sets one.
pub const DEFAULT_CHAIN: &[&str] = &["env", "keychain"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainEntry {
    Env,
    Keychain,
    /// A reference template containing `{name}`; must not be `secret://`.
    Template(String),
}

impl FromStr for ChainEntry {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "env" => Ok(ChainEntry::Env),
            "keychain" => Ok(ChainEntry::Keychain),
            t if t.contains("{name}") => {
                let probe: SecretRef = t
                    .replace("{name}", "probe/name")
                    .parse()
                    .map_err(|e| format!("secret chain entry {t:?}: {e}"))?;
                if matches!(probe, SecretRef::Named(_)) {
                    return Err(format!(
                        "secret chain entry {t:?} cannot be a secret:// reference"
                    ));
                }
                Ok(ChainEntry::Template(t.to_owned()))
            }
            other => Err(format!(
                "unknown secret chain entry {other:?} (use env, keychain, or a reference template with {{name}})"
            )),
        }
    }
}

impl fmt::Display for ChainEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChainEntry::Env => f.write_str("env"),
            ChainEntry::Keychain => f.write_str("keychain"),
            ChainEntry::Template(t) => f.write_str(t),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain(pub Vec<ChainEntry>);

impl Default for Chain {
    fn default() -> Self {
        Chain(vec![ChainEntry::Env, ChainEntry::Keychain])
    }
}

impl Chain {
    pub fn parse<S: AsRef<str>>(entries: &[S]) -> Result<Self, String> {
        if entries.is_empty() {
            return Err("the secret chain must not be empty".into());
        }
        entries
            .iter()
            .map(|e| e.as_ref().parse())
            .collect::<Result<_, _>>()
            .map(Chain)
    }

    /// The reference entry `e` tries for `name`.
    pub fn candidate(e: &ChainEntry, name: &str) -> SecretRef {
        match e {
            ChainEntry::Env => SecretRef::Env(env_var_for(name)),
            ChainEntry::Keychain => SecretRef::Keychain {
                service: KEYCHAIN_SERVICE.into(),
                account: name.into(),
            },
            ChainEntry::Template(t) => t
                .replace("{name}", name)
                .parse()
                .unwrap_or_else(|_| SecretRef::Env(env_var_for(name))),
        }
    }

    pub(crate) fn resolve(
        &self,
        name: &str,
        res: &Resolvers<'_>,
    ) -> Result<SecretString, SecretError> {
        let mut tried = Vec::new();
        for e in &self.0 {
            let r = Self::candidate(e, name);
            if matches!(r, SecretRef::Named(_)) {
                continue;
            }
            match res.resolve_direct(&r) {
                Ok(v) => return Ok(v),
                Err(err) => tried.push(match err {
                    SecretError::NotFound { detail, .. } => format!("{r}: {detail}"),
                    other => other.to_string(),
                }),
            }
        }
        Err(SecretError::NotFound {
            reference: format!("secret://{name}"),
            detail: format!("tried {}", tried.join("; ")),
        })
    }
}

/// `jira/token` → `JIRA_TOKEN`: uppercase, every other character `_`.
pub fn env_var_for(name: &str) -> String {
    let mut v: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    if v.starts_with(|c: char| c.is_ascii_digit()) {
        v.insert(0, '_');
    }
    v
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;
    use crate::testutil::{FakeRunner, MemKeystore};
    use crate::{Keystore, SecretResolver};

    #[test]
    fn env_names() {
        assert_eq!(env_var_for("jira/token"), "JIRA_TOKEN");
        assert_eq!(env_var_for("a.b-c"), "A_B_C");
        assert_eq!(env_var_for("1x"), "_1X");
    }

    #[test]
    fn parses_entries() {
        let c = Chain::parse(&["env", "keychain", "op://Acme/{name}/credential"]).unwrap();
        assert_eq!(c.0.len(), 3);
        assert_eq!(c.0[2].to_string(), "op://Acme/{name}/credential");
        assert!(Chain::parse(&["ldap"]).is_err());
        assert!(Chain::parse(&["secret://{name}"]).is_err());
        assert!(Chain::parse(&["vault://{name}"]).is_err()); // no #field
        assert!(Chain::parse::<&str>(&[]).is_err());
    }

    #[test]
    fn tries_entries_in_order() {
        let ks = MemKeystore::default();
        let runner = FakeRunner::default().answer(
            "op read --no-newline op://Acme/jira/token/credential",
            0,
            "from-op",
            "",
        );
        let chain = Chain::parse(&["env", "keychain", "op://Acme/{name}/credential"]).unwrap();
        let mut env_val: Option<String> = None;
        let run = |env_val: &Option<String>| {
            let e = env_val.clone();
            let lookup = move |k: &str| (k == "JIRA_TOKEN").then(|| e.clone()).flatten();
            let res = Resolvers {
                env: &lookup,
                keystore: &ks,
                runner: &runner,
                chain: &chain,
                source_dir: None,
            };
            res.resolve(&SecretRef::Named("jira/token".into()))
                .map(|v| v.expose_secret().to_owned())
        };
        assert_eq!(run(&env_val).unwrap(), "from-op");
        ks.set("loadout", "jira/token", &SecretString::from("from-kc"))
            .unwrap();
        assert_eq!(run(&env_val).unwrap(), "from-kc");
        env_val = Some("from-env".into());
        assert_eq!(run(&env_val).unwrap(), "from-env");
    }

    #[test]
    fn reports_every_attempt() {
        let ks = MemKeystore::default();
        let runner = FakeRunner::default();
        let chain = Chain::default();
        let lookup = |_: &str| None;
        let res = Resolvers {
            env: &lookup,
            keystore: &ks,
            runner: &runner,
            chain: &chain,
            source_dir: None,
        };
        let e = res
            .resolve(&SecretRef::Named("jira/token".into()))
            .unwrap_err();
        assert!(e.is_not_found());
        assert_eq!(
            e.to_string(),
            "secret://jira/token: not found (tried env://JIRA_TOKEN: environment variable JIRA_TOKEN is not set; keychain://loadout/jira/token: no keychain entry)"
        );
    }
}

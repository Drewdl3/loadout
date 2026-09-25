//! Secret resolvers: turn references such as `secret://jira/token`
//! into values at the moment they are needed, without storing them.
//!
//! Values are held in [`SecretString`] and never logged. Error messages name
//! references, never values.

pub mod chain;
pub mod keystore;
pub mod runner;
pub mod tools;

use std::path::PathBuf;

use loadout_model::SecretRef;
pub use secrecy::{ExposeSecret, SecretString};

pub use chain::{Chain, ChainEntry, DEFAULT_CHAIN};
pub use keystore::{KEYCHAIN_SERVICE, Keystore, OsKeystore, default_keystore};
pub use runner::{Runner, SystemRunner};

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// The reference points at nothing (unset variable, no keychain entry…).
    #[error("{reference}: not found ({detail})")]
    NotFound { reference: String, detail: String },
    /// A helper tool (`op`, `vault`, `sops`) is not installed.
    #[error("{reference}: `{tool}` is not installed or not on PATH")]
    ToolMissing { reference: String, tool: String },
    /// A helper tool or the keychain failed.
    #[error("{reference}: {detail}")]
    Failed { reference: String, detail: String },
    /// `sops://` needs the checkout of the item's source.
    #[error("{reference}: sops references need the item's source checkout")]
    NoSourceDir { reference: String },
}

impl SecretError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, SecretError::NotFound { .. })
    }
}

/// Resolves one reference to its value.
pub trait SecretResolver {
    fn resolve(&self, reference: &SecretRef) -> Result<SecretString, SecretError>;
}

/// Everything resolution needs from the outside world, injected so tests
/// never touch the real environment, keychain or tools.
pub struct Resolvers<'a> {
    /// Environment lookup.
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub keystore: &'a dyn Keystore,
    pub runner: &'a dyn Runner,
    /// How `secret://name` is resolved.
    pub chain: &'a Chain,
    /// Checkout of the item's source repo, for `sops://`.
    pub source_dir: Option<PathBuf>,
}

impl SecretResolver for Resolvers<'_> {
    fn resolve(&self, reference: &SecretRef) -> Result<SecretString, SecretError> {
        match reference {
            SecretRef::Named(name) => self.chain.resolve(name, self),
            other => self.resolve_direct(other),
        }
    }
}

impl Resolvers<'_> {
    /// Resolves a reference that is not `secret://`.
    fn resolve_direct(&self, r: &SecretRef) -> Result<SecretString, SecretError> {
        let reference = r.to_string();
        match r {
            SecretRef::Named(_) => unreachable!("handled by the chain"),
            SecretRef::Env(var) => match (self.env)(var) {
                Some(v) if !v.is_empty() => Ok(SecretString::from(v)),
                _ => Err(SecretError::NotFound {
                    reference,
                    detail: format!("environment variable {var} is not set"),
                }),
            },
            SecretRef::Keychain { service, account } => match self.keystore.get(service, account) {
                Ok(Some(v)) => Ok(v),
                Ok(None) => Err(SecretError::NotFound {
                    reference,
                    detail: "no keychain entry".into(),
                }),
                Err(detail) => Err(SecretError::Failed { reference, detail }),
            },
            SecretRef::Op(_) => tools::op(self.runner, &reference),
            SecretRef::Vault { path, field } => tools::vault(self.runner, &reference, path, field),
            SecretRef::Sops { path, key } => {
                let Some(dir) = &self.source_dir else {
                    return Err(SecretError::NoSourceDir { reference });
                };
                tools::sops(self.runner, &reference, dir, path, key)
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::*;
    use crate::runner::RunOutput;

    #[derive(Default)]
    pub struct MemKeystore(pub RefCell<BTreeMap<(String, String), String>>);

    impl Keystore for MemKeystore {
        fn get(&self, service: &str, account: &str) -> Result<Option<SecretString>, String> {
            Ok(self
                .0
                .borrow()
                .get(&(service.into(), account.into()))
                .map(|v| SecretString::from(v.clone())))
        }
        fn set(&self, service: &str, account: &str, value: &SecretString) -> Result<(), String> {
            self.0.borrow_mut().insert(
                (service.into(), account.into()),
                value.expose_secret().to_owned(),
            );
            Ok(())
        }
        fn delete(&self, service: &str, account: &str) -> Result<bool, String> {
            Ok(self
                .0
                .borrow_mut()
                .remove(&(service.into(), account.into()))
                .is_some())
        }
    }

    /// Answers `program args…` with canned outputs; records calls.
    #[derive(Default)]
    pub struct FakeRunner {
        pub answers: BTreeMap<String, RunOutput>,
        pub calls: RefCell<Vec<String>>,
    }

    impl FakeRunner {
        pub fn answer(mut self, cmdline: &str, status: i32, stdout: &str, stderr: &str) -> Self {
            self.answers.insert(
                cmdline.into(),
                RunOutput {
                    success: status == 0,
                    stdout: stdout.as_bytes().to_vec(),
                    stderr: stderr.into(),
                },
            );
            self
        }
    }

    impl Runner for FakeRunner {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            cwd: Option<&Path>,
        ) -> std::io::Result<RunOutput> {
            let mut line = std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(cwd) = cwd {
                line = format!("[{}] {line}", cwd.file_name().unwrap().to_string_lossy());
            }
            self.calls.borrow_mut().push(line.clone());
            self.answers
                .get(&line)
                .cloned()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no such tool"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    fn resolve(
        r: &str,
        env: &[(&str, &str)],
        ks: &MemKeystore,
        runner: &FakeRunner,
    ) -> Result<String, SecretError> {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let lookup = move |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        let chain = Chain::default();
        let res = Resolvers {
            env: &lookup,
            keystore: ks,
            runner,
            chain: &chain,
            source_dir: None,
        };
        res.resolve(&r.parse().unwrap())
            .map(|s| s.expose_secret().to_owned())
    }

    #[test]
    fn env_and_keychain() {
        let ks = MemKeystore::default();
        let runner = FakeRunner::default();
        assert_eq!(
            resolve("env://T", &[("T", "v")], &ks, &runner).unwrap(),
            "v"
        );
        assert!(
            resolve("env://T", &[], &ks, &runner)
                .unwrap_err()
                .is_not_found()
        );
        assert!(
            resolve("env://T", &[("T", "")], &ks, &runner)
                .unwrap_err()
                .is_not_found()
        );

        ks.set("acme", "jira", &SecretString::from("k")).unwrap();
        assert_eq!(
            resolve("keychain://acme/jira", &[], &ks, &runner).unwrap(),
            "k"
        );
        let e = resolve("keychain://acme/none", &[], &ks, &runner).unwrap_err();
        assert!(e.is_not_found());
        assert_eq!(
            e.to_string(),
            "keychain://acme/none: not found (no keychain entry)"
        );
    }

    #[test]
    fn sops_without_source_dir_is_an_error() {
        let e = resolve(
            "sops://s.yaml#k",
            &[],
            &MemKeystore::default(),
            &FakeRunner::default(),
        )
        .unwrap_err();
        assert!(matches!(e, SecretError::NoSourceDir { .. }));
    }
}

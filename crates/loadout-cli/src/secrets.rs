//! Wiring for secret resolution: the configured chain and the
//! real environment, keychain and tools.

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use loadout_model::{CompanyConfig, Config};
use loadout_secrets::{Chain, DEFAULT_CHAIN, Keystore, Resolvers, SystemRunner};

/// The `secret://` chain: `[secrets] chain` in config.toml, else the
/// company config's `secrets.chain`, else [`DEFAULT_CHAIN`].
pub fn chain(config: &Config, company_config: Option<&CompanyConfig>) -> Result<Chain> {
    let (entries, origin): (Vec<String>, &str) = if let Some(c) = &config.secrets.chain {
        (c.clone(), "[secrets] chain in config.toml")
    } else if let Some(c) = company_config
        .and_then(|c| c.secrets.as_ref())
        .filter(|s| !s.chain.is_empty())
    {
        (c.chain.clone(), "the company config's secrets.chain")
    } else {
        (DEFAULT_CHAIN.iter().map(|s| (*s).to_owned()).collect(), "")
    };
    Chain::parse(&entries).map_err(|e| anyhow!("{e} (in {origin})"))
}

/// The real environment, keychain and tools.
pub struct System {
    pub chain: Chain,
    pub keystore: Box<dyn Keystore>,
    pub runner: SystemRunner,
}

pub fn env_lookup(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

impl System {
    pub fn new(chain: Chain) -> Self {
        System {
            chain,
            keystore: loadout_secrets::default_keystore(),
            runner: SystemRunner,
        }
    }

    /// Resolvers for items of a source checked out at `source_dir`.
    pub fn resolvers(&self, source_dir: Option<PathBuf>) -> Resolvers<'_> {
        Resolvers {
            env: &env_lookup,
            keystore: self.keystore.as_ref(),
            runner: &self.runner,
            chain: &self.chain,
            source_dir,
        }
    }
}

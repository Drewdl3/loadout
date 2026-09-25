//! The OS keychain (`keychain://`): macOS Keychain, Windows
//! Credential Manager, or the Secret Service on Linux, via `keyring`.

use secrecy::{ExposeSecret, SecretString};

/// Keychain service under which `secret://<name>` values are stored by
/// `lo secrets set` (account = the secret name).
pub const KEYCHAIN_SERVICE: &str = "loadout";

/// A credential store. Errors are human-readable messages without values.
pub trait Keystore {
    fn get(&self, service: &str, account: &str) -> Result<Option<SecretString>, String>;
    fn set(&self, service: &str, account: &str, value: &SecretString) -> Result<(), String>;
    /// Returns whether an entry existed.
    fn delete(&self, service: &str, account: &str) -> Result<bool, String>;
}

/// The platform keychain.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeystore;

impl Keystore for OsKeystore {
    fn get(&self, service: &str, account: &str) -> Result<Option<SecretString>, String> {
        let entry = keyring::Entry::new(service, account).map_err(describe)?;
        match entry.get_password() {
            Ok(v) => Ok(Some(SecretString::from(v))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(describe(e)),
        }
    }

    fn set(&self, service: &str, account: &str, value: &SecretString) -> Result<(), String> {
        keyring::Entry::new(service, account)
            .and_then(|e| e.set_password(value.expose_secret()))
            .map_err(describe)
    }

    fn delete(&self, service: &str, account: &str) -> Result<bool, String> {
        let entry = keyring::Entry::new(service, account).map_err(describe)?;
        match entry.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(describe(e)),
        }
    }
}

fn describe(e: keyring::Error) -> String {
    format!("keychain: {e}")
}

/// A JSON file standing in for the keychain in end-to-end tests
/// (`LOADOUT_TEST_KEYSTORE`). Plain text; tests only.
#[cfg(feature = "test-support")]
#[derive(Debug, Clone)]
pub struct FileKeystore(pub std::path::PathBuf);

#[cfg(feature = "test-support")]
impl FileKeystore {
    fn load(&self) -> Result<serde_json::Map<String, serde_json::Value>, String> {
        match std::fs::read_to_string(&self.0) {
            Ok(t) => serde_json::from_str(&t).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn save(&self, map: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
        std::fs::write(&self.0, serde_json::to_string(map).unwrap()).map_err(|e| e.to_string())
    }
}

#[cfg(feature = "test-support")]
impl Keystore for FileKeystore {
    fn get(&self, service: &str, account: &str) -> Result<Option<SecretString>, String> {
        Ok(self
            .load()?
            .get(&format!("{service}/{account}"))
            .and_then(|v| v.as_str())
            .map(|v| SecretString::from(v.to_owned())))
    }

    fn set(&self, service: &str, account: &str, value: &SecretString) -> Result<(), String> {
        let mut m = self.load()?;
        m.insert(
            format!("{service}/{account}"),
            value.expose_secret().to_owned().into(),
        );
        self.save(&m)
    }

    fn delete(&self, service: &str, account: &str) -> Result<bool, String> {
        let mut m = self.load()?;
        let existed = m.remove(&format!("{service}/{account}")).is_some();
        self.save(&m)?;
        Ok(existed)
    }
}

/// The keystore to use: the OS keychain, or with the `test-support` feature
/// the file named by `LOADOUT_TEST_KEYSTORE`.
pub fn default_keystore() -> Box<dyn Keystore> {
    #[cfg(feature = "test-support")]
    if let Some(p) = std::env::var_os("LOADOUT_TEST_KEYSTORE") {
        return Box::new(FileKeystore(p.into()));
    }
    Box::new(OsKeystore)
}

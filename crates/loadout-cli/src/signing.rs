//! Signed-commit verification: the company config's allowed signers,
//! prepared for `git verify-commit`.

use std::io::Write as _;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use loadout_git::SignerKeys;
use loadout_model::CompanyConfig;

/// Verification setup for one apply. Keeps its temporary key files alive.
pub struct Signing {
    /// Layers whose commits must be signed.
    pub layers: Vec<String>,
    pub keys: SignerKeys,
    /// A setup problem to warn about (verification then fails closed).
    pub problem: Option<String>,
    _dir: tempfile::TempDir,
}

impl Signing {
    pub fn requires(&self, layer: &str) -> bool {
        self.layers.iter().any(|s| s == layer)
    }
}

/// `None` when the company config requires no signatures.
pub fn prepare(company_config: Option<&CompanyConfig>) -> Result<Option<Signing>> {
    let Some(c) = company_config.filter(|c| !c.policy.require_signed.is_empty()) else {
        return Ok(None);
    };
    let dir = tempfile::Builder::new()
        .prefix("loadout-signers-")
        .tempdir()
        .context("creating a temporary directory for allowed signers")?;
    let mut keys = SignerKeys::default();
    let mut problem = None;
    let ssh: Vec<String> = c
        .signers
        .iter()
        .filter_map(|s| {
            s.ssh
                .as_ref()
                .map(|k| format!("{} {}\n", s.principal, k.trim()))
        })
        .collect();
    if !ssh.is_empty() {
        let file = dir.path().join("allowed_signers");
        std::fs::write(&file, ssh.concat()).context("writing allowed_signers")?;
        keys.allowed_signers = Some(file);
    }
    let gpg: Vec<&String> = c.signers.iter().filter_map(|s| s.gpg.as_ref()).collect();
    if !gpg.is_empty() {
        let home = dir.path().join("gnupg");
        std::fs::create_dir_all(&home)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700))?;
        }
        for key in gpg {
            if let Err(e) = gpg_import(&home, key) {
                problem = Some(format!(
                    "could not import the company config's GPG signer keys ({e}); GPG-signed commits will be refused"
                ));
                break;
            }
        }
        keys.gnupg_home = Some(home);
    }
    if c.signers.is_empty() {
        problem = Some(format!(
            "the company config requires signed commits for {} but lists no signers; updates in those layers will be refused",
            c.policy.require_signed.join(", ")
        ));
    }
    Ok(Some(Signing {
        layers: c.policy.require_signed.clone(),
        keys,
        problem,
        _dir: dir,
    }))
}

fn gpg_import(home: &std::path::Path, key: &str) -> Result<()> {
    let mut child = Command::new("gpg")
        .arg("--homedir")
        .arg(home)
        .args(["--batch", "--quiet", "--import"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("running gpg")?;
    child
        .stdin
        .take()
        .context("gpg stdin")?
        .write_all(key.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

//! `exec` provider: run a company-provided command that prints the user's
//! groups as JSON (an array of strings, or `{"groups": [...]}`), bridging
//! Okta/Entra/LDAP without Loadout implementing IdPs.

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Mutex;

use crate::{GroupCommand, ProviderError};

/// Runs commands through the platform shell, caching each command's
/// output for the lifetime of the value (one discovery run).
#[derive(Debug, Default)]
pub struct ShellGroups {
    cache: Mutex<BTreeMap<String, Result<Vec<String>, ProviderError>>>,
}

impl GroupCommand for ShellGroups {
    fn groups(&self, command: &str) -> Result<Vec<String>, ProviderError> {
        let mut cache = self.cache.lock().expect("exec cache poisoned");
        cache
            .entry(command.to_owned())
            .or_insert_with(|| run(command))
            .clone()
    }
}

#[cfg(windows)]
fn shell(command: &str) -> Command {
    use std::os::windows::process::CommandExt;
    // cmd.exe does its own parsing; pass the command verbatim so quotes
    // aren't escaped the way `Command::arg` does for C runtimes.
    let mut c = Command::new("cmd");
    c.arg("/C").raw_arg(command);
    c
}

#[cfg(not(windows))]
fn shell(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.args(["-c", command]);
    c
}

fn run(command: &str) -> Result<Vec<String>, ProviderError> {
    let err = |m: String| ProviderError::new("exec", m);
    let out = shell(command)
        .output()
        .map_err(|e| err(format!("could not run {command:?}: {e}")))?;
    if !out.status.success() {
        return Err(err(format!(
            "{command:?} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    parse_groups(&out.stdout).map_err(|m| err(format!("{command:?}: {m}")))
}

/// Accepts `["a", "b"]` or `{"groups": ["a", "b"]}`.
pub fn parse_groups(stdout: &[u8]) -> Result<Vec<String>, String> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Out {
        List(Vec<String>),
        Obj { groups: Vec<String> },
    }
    match serde_json::from_slice::<Out>(stdout) {
        Ok(Out::List(g) | Out::Obj { groups: g }) => Ok(g),
        Err(e) => Err(format!(
            "expected a JSON array of group names or {{\"groups\": [...]}} ({e})"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_shapes() {
        assert_eq!(parse_groups(br#"["a","b"]"#).unwrap(), ["a", "b"]);
        assert_eq!(parse_groups(br#"{"groups":["x"]}"#).unwrap(), ["x"]);
        assert!(parse_groups(b"not json").is_err());
    }

    #[test]
    fn runs_command_and_caches() {
        let g = ShellGroups::default();
        let cmd = r#"echo ["role:sre","eng"]"#;
        let cmd = if cfg!(windows) {
            cmd.to_owned()
        } else {
            format!("echo '{}'", &cmd[5..])
        };
        assert_eq!(g.groups(&cmd).unwrap(), ["role:sre", "eng"]);
        assert_eq!(g.cache.lock().unwrap().len(), 1);
        g.groups(&cmd).unwrap();
        assert_eq!(g.cache.lock().unwrap().len(), 1);
    }

    #[test]
    fn failing_command_is_an_error() {
        let g = ShellGroups::default();
        assert!(g.groups("exit 3").is_err());
    }
}

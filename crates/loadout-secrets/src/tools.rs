//! Resolvers that shell out: 1Password (`op`), HashiCorp Vault (`vault`),
//! SOPS (`sops`).

use std::path::Path;

use secrecy::SecretString;

use crate::SecretError;
use crate::runner::Runner;

/// `op read <uri>`.
pub fn op(runner: &dyn Runner, reference: &str) -> Result<SecretString, SecretError> {
    run_tool(
        runner,
        reference,
        "op",
        &["read", "--no-newline", reference],
        None,
    )
}

/// `vault kv get -field=<field> <path>`.
pub fn vault(
    runner: &dyn Runner,
    reference: &str,
    path: &str,
    field: &str,
) -> Result<SecretString, SecretError> {
    let field_arg = format!("-field={field}");
    run_tool(
        runner,
        reference,
        "vault",
        &["kv", "get", &field_arg, path],
        None,
    )
}

/// `sops --decrypt --extract '["a"]["b"]' <path>` inside the source
/// checkout; `key` is a dotted path into the document.
pub fn sops(
    runner: &dyn Runner,
    reference: &str,
    source_dir: &Path,
    path: &str,
    key: &str,
) -> Result<SecretString, SecretError> {
    let file = source_dir.join(path);
    if !file.is_file() {
        return Err(SecretError::NotFound {
            reference: reference.to_owned(),
            detail: format!("{path} does not exist in the source repo"),
        });
    }
    let extract: String = key
        .split('.')
        .map(|k| {
            serde_json::to_string(k)
                .map(|q| format!("[{q}]"))
                .unwrap_or_default()
        })
        .collect();
    run_tool(
        runner,
        reference,
        "sops",
        &["--decrypt", "--extract", &extract, path],
        Some(source_dir),
    )
}

fn run_tool(
    runner: &dyn Runner,
    reference: &str,
    tool: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<SecretString, SecretError> {
    let out = runner.run(tool, args, cwd).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SecretError::ToolMissing {
                reference: reference.to_owned(),
                tool: tool.to_owned(),
            }
        } else {
            SecretError::Failed {
                reference: reference.to_owned(),
                detail: format!("running {tool}: {e}"),
            }
        }
    })?;
    if !out.success {
        let first = out
            .stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        return Err(SecretError::Failed {
            reference: reference.to_owned(),
            detail: format!("{tool} failed: {}", first.trim()),
        });
    }
    let text = String::from_utf8(out.stdout).map_err(|_| SecretError::Failed {
        reference: reference.to_owned(),
        detail: format!("{tool} returned a non-UTF-8 value"),
    })?;
    let value = text.strip_suffix('\n').unwrap_or(&text);
    let value = value.strip_suffix('\r').unwrap_or(value);
    if value.is_empty() {
        return Err(SecretError::NotFound {
            reference: reference.to_owned(),
            detail: format!("{tool} returned an empty value"),
        });
    }
    Ok(SecretString::from(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;
    use crate::testutil::FakeRunner;

    #[test]
    fn op_reads_value() {
        let r = FakeRunner::default().answer(
            "op read --no-newline op://Eng/Jira/token",
            0,
            "s3cr3t",
            "",
        );
        let v = op(&r, "op://Eng/Jira/token").unwrap();
        assert_eq!(v.expose_secret(), "s3cr3t");
    }

    #[test]
    fn vault_trims_newline_and_reports_failures() {
        let r = FakeRunner::default()
            .answer("vault kv get -field=token secret/jira", 0, "v\n", "")
            .answer(
                "vault kv get -field=token secret/none",
                2,
                "",
                "\nNo value found at secret/data/none\n",
            );
        assert_eq!(
            vault(&r, "vault://secret/jira#token", "secret/jira", "token")
                .unwrap()
                .expose_secret(),
            "v"
        );
        let e = vault(&r, "vault://secret/none#token", "secret/none", "token").unwrap_err();
        assert_eq!(
            e.to_string(),
            "vault://secret/none#token: vault failed: No value found at secret/data/none"
        );
    }

    #[test]
    fn missing_tool_is_reported() {
        let e = op(&FakeRunner::default(), "op://a/b/c").unwrap_err();
        assert!(matches!(e, SecretError::ToolMissing { ref tool, .. } if tool == "op"));
    }

    #[test]
    fn sops_extracts_nested_key_in_source_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("secrets")).unwrap();
        std::fs::write(src.join("secrets/jira.enc.yaml"), "enc").unwrap();
        let r = FakeRunner::default().answer(
            r#"[src] sops --decrypt --extract ["jira"]["token"] secrets/jira.enc.yaml"#,
            0,
            "t0k",
            "",
        );
        let v = sops(&r, "sops://…", &src, "secrets/jira.enc.yaml", "jira.token").unwrap();
        assert_eq!(v.expose_secret(), "t0k");
        let e = sops(&r, "sops://…", &src, "secrets/missing.yaml", "k").unwrap_err();
        assert!(e.is_not_found());
    }

    #[test]
    fn empty_output_is_not_found() {
        let r = FakeRunner::default().answer("op read --no-newline op://a/b/c", 0, "", "");
        assert!(op(&r, "op://a/b/c").unwrap_err().is_not_found());
    }
}

//! Loadout Git clone/fetch/checkout, driven through the `git` CLI.
//!
//! Using the user's `git` means existing credential helpers, SSH config and
//! proxies work unchanged (repo access = Git access).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[cfg(feature = "test-support")]
pub mod fixture;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("could not run `{program}`: {source} (is Git installed and on PATH?)")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`git {args}` failed ({status}): {stderr}")]
    Failed {
        args: String,
        status: String,
        stderr: String,
    },
    #[error("unexpected output from `git {args}`: {output:?}")]
    BadOutput { args: String, output: String },
}

/// A full commit SHA.
pub type CommitId = String;

/// Keys a signed commit must match (see [`Git::verify_commit`]).
#[derive(Debug, Clone, Default)]
pub struct SignerKeys {
    /// An SSH `allowed_signers` file.
    pub allowed_signers: Option<PathBuf>,
    /// A GnuPG home directory holding the allowed public keys.
    pub gnupg_home: Option<PathBuf>,
}

/// Handle to the `git` executable plus environment for every invocation.
#[derive(Debug, Clone)]
pub struct Git {
    program: OsString,
    envs: Vec<(OsString, OsString)>,
}

impl Default for Git {
    fn default() -> Self {
        Git::new()
    }
}

impl Git {
    /// Uses `git` from `PATH`, non-interactively (no credential prompts on
    /// the terminal, so scheduled syncs never hang).
    pub fn new() -> Self {
        Git {
            program: "git".into(),
            envs: vec![("GIT_TERMINAL_PROMPT".into(), "0".into())],
        }
    }

    /// Adds an environment variable to every invocation.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// `git --version`, e.g. `git version 2.43.0`.
    pub fn version(&self) -> Result<String, GitError> {
        let out = self.run(None, ["--version"])?;
        Ok(out.trim().to_owned())
    }

    /// Makes `dir` a checkout of `url` at `git_ref` (default branch when
    /// `None`) and returns the checked-out commit: [`Git::fetch`] then
    /// [`Git::checkout`].
    pub fn sync_checkout(
        &self,
        url: &str,
        dir: &Path,
        git_ref: Option<&str>,
    ) -> Result<CommitId, GitError> {
        let commit = self.fetch(url, dir, git_ref)?;
        self.checkout(dir, &commit)?;
        Ok(commit)
    }

    /// Fetches `git_ref` (default branch when `None`) of `url` into the
    /// repo at `dir` (created if needed) without changing the working tree,
    /// and returns the fetched commit. The fetch is shallow.
    pub fn fetch(
        &self,
        url: &str,
        dir: &Path,
        git_ref: Option<&str>,
    ) -> Result<CommitId, GitError> {
        self.ensure_repo(dir)?;
        let refspec = git_ref.unwrap_or("HEAD");
        self.run(
            Some(dir),
            [
                "fetch",
                "--quiet",
                "--no-tags",
                "--depth",
                "1",
                &fetch_url(url),
                refspec,
            ],
        )?;
        let commit = self.rev_parse(dir, "FETCH_HEAD^{commit}")?;
        // Keep the object reachable so it survives `git gc`.
        self.run(Some(dir), ["update-ref", "refs/loadout/fetched", &commit])?;
        Ok(commit)
    }

    /// Fetches exactly `commit` from `url` (for pinned installs on a machine
    /// that doesn't have it yet). Most hosts allow fetching by SHA.
    pub fn fetch_commit(&self, url: &str, dir: &Path, commit: &str) -> Result<(), GitError> {
        self.ensure_repo(dir)?;
        self.run(
            Some(dir),
            [
                "fetch",
                "--quiet",
                "--no-tags",
                "--depth",
                "1",
                &fetch_url(url),
                commit,
            ],
        )?;
        Ok(())
    }

    /// Forces the working tree of `dir` to `commit` exactly (local changes
    /// and untracked files are discarded — the clone is Loadout-owned).
    pub fn checkout(&self, dir: &Path, commit: &str) -> Result<(), GitError> {
        self.run(
            Some(dir),
            ["checkout", "--quiet", "--force", "--detach", commit],
        )?;
        self.run(Some(dir), ["clean", "--quiet", "-ffdx"])?;
        self.run(Some(dir), ["update-ref", "refs/loadout/applied", commit])?;
        Ok(())
    }

    /// Whether the repo at `dir` has `commit`.
    pub fn has_commit(&self, dir: &Path, commit: &str) -> bool {
        dir.join(".git").exists()
            && self
                .run(
                    Some(dir),
                    ["cat-file", "-e", &format!("{commit}^{{commit}}")],
                )
                .is_ok()
    }

    /// Verifies `commit`'s GPG or SSH signature against the allowed keys:
    /// an SSH `allowed_signers` file and/or a GnuPG home containing only the
    /// allowed public keys. Returns git's report on success.
    pub fn verify_commit(
        &self,
        dir: &Path,
        commit: &str,
        keys: &SignerKeys,
    ) -> Result<String, GitError> {
        let mut git = self.clone();
        if let Some(home) = &keys.gnupg_home {
            git = git.env("GNUPGHOME", home);
        }
        let mut args: Vec<OsString> = Vec::new();
        if let Some(file) = &keys.allowed_signers {
            args.push("-c".into());
            let mut kv = OsString::from("gpg.ssh.allowedSignersFile=");
            kv.push(file);
            args.push(kv);
        }
        args.extend(["verify-commit".into(), "--verbose".into(), commit.into()]);
        let mut cmd = git.command(Some(dir));
        cmd.args(&args);
        let out = cmd.output().map_err(|source| GitError::Spawn {
            program: self.program.to_string_lossy().into_owned(),
            source,
        })?;
        // verify-commit reports on stderr.
        let report = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        if out.status.success() {
            Ok(report)
        } else {
            Err(GitError::Failed {
                args: format!("verify-commit {commit}"),
                status: out.status.to_string(),
                stderr: if report.is_empty() {
                    "no signature".into()
                } else {
                    report
                },
            })
        }
    }

    /// `git diff <from> <to>` (full patch with rename detection).
    pub fn diff(&self, dir: &Path, from: &str, to: &str) -> Result<String, GitError> {
        self.run(
            Some(dir),
            ["diff", "--no-color", "--no-ext-diff", "-M", from, to],
        )
    }

    fn ensure_repo(&self, dir: &Path) -> Result<(), GitError> {
        if !dir.join(".git").exists() {
            std::fs::create_dir_all(dir).map_err(|source| GitError::Spawn {
                program: format!("mkdir {}", dir.display()),
                source,
            })?;
            self.run(Some(dir), ["init", "--quiet"])?;
        }
        Ok(())
    }

    fn rev_parse(&self, dir: &Path, rev: &str) -> Result<CommitId, GitError> {
        let out = self.run(Some(dir), ["rev-parse", rev])?;
        let sha = out.trim();
        if sha.len() >= 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(sha.to_owned())
        } else {
            Err(GitError::BadOutput {
                args: format!("rev-parse {rev}"),
                output: out,
            })
        }
    }

    /// Whether `url` can be read with the user's credentials
    /// (`git ls-remote`). A failed read is `Err(GitError::Failed)`.
    pub fn can_read(&self, url: &str) -> Result<bool, GitError> {
        self.run(None, ["ls-remote", "--quiet", &fetch_url(url), "HEAD"])
            .map(|_| true)
    }

    /// Runs git with `args`, feeding `stdin`, returning stdout on success.
    pub fn run_with_stdin<I, S>(
        &self,
        dir: Option<&Path>,
        args: I,
        stdin: &[u8],
    ) -> Result<String, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        use std::io::Write;
        use std::process::Stdio;
        let args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
        let mut cmd = self.command(dir);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let spawn_err = |source| GitError::Spawn {
            program: self.program.to_string_lossy().into_owned(),
            source,
        };
        let mut child = cmd.spawn().map_err(spawn_err)?;
        if let Some(mut input) = child.stdin.take() {
            input.write_all(stdin).map_err(spawn_err)?;
        }
        let out = child.wait_with_output().map_err(spawn_err)?;
        Self::finish(&args, out)
    }

    /// The commit checked out in `dir`.
    pub fn head(&self, dir: &Path) -> Result<CommitId, GitError> {
        self.rev_parse(dir, "HEAD")
    }

    fn command(&self, dir: Option<&Path>) -> Command {
        let mut cmd = Command::new(&self.program);
        if let Some(dir) = dir {
            cmd.arg("-C").arg(dir);
        }
        cmd.envs(self.envs.iter().map(|(k, v)| (k, v)));
        cmd
    }

    /// Runs git with `args`, returning stdout on success.
    pub fn run<I, S>(&self, dir: Option<&Path>, args: I) -> Result<String, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
        let mut cmd = self.command(dir);
        cmd.args(&args);
        tracing::debug!(?args, ?dir, "git");
        let out: Output = cmd.output().map_err(|source| GitError::Spawn {
            program: self.program.to_string_lossy().into_owned(),
            source,
        })?;
        Self::finish(&args, out)
    }

    fn finish(args: &[OsString], out: Output) -> Result<String, GitError> {
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(GitError::Failed {
                args: args
                    .iter()
                    .map(|a| a.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" "),
                status: out.status.to_string(),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            })
        }
    }
}

/// Local paths are fetched through `file://` so `--depth` is honoured.
fn fetch_url(url: &str) -> String {
    let p = Path::new(url);
    if p.is_absolute() && p.exists() {
        file_url(p)
    } else {
        url.to_owned()
    }
}

fn file_url(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        // Windows drive path: C:/x -> file:///C:/x
        format!("file:///{s}")
    }
}

/// Directory name for a source's clone: first 16 hex chars of
/// `blake3(url)`. Callers pass a normalized URL.
pub fn repo_dir_name(url: &str) -> String {
    let hash = blake3::hash(url.as_bytes()).to_hex();
    hash[..16].to_owned()
}

/// `repos/<blake3(url)[:16]>`.
pub fn repo_dir(repos_root: &Path, url: &str) -> PathBuf {
    repos_root.join(repo_dir_name(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_dir_name_is_stable_hex() {
        let n = repo_dir_name("https://example.com/acme/skills");
        assert_eq!(n.len(), 16);
        assert!(n.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(n, repo_dir_name("https://example.com/acme/skills"));
        assert_ne!(n, repo_dir_name("https://example.com/acme/other"));
    }

    #[test]
    fn file_urls() {
        assert_eq!(file_url(Path::new("/tmp/r")), "file:///tmp/r");
        assert_eq!(file_url(Path::new(r"C:\t\r")), "file:///C:/t/r");
        assert_eq!(fetch_url("https://example.com/x"), "https://example.com/x");
    }

    #[test]
    fn missing_program_is_spawn_error() {
        let git = Git {
            program: "definitely-not-git-loadout".into(),
            envs: vec![],
        };
        assert!(matches!(git.version(), Err(GitError::Spawn { .. })));
    }
}

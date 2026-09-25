//! Local fixture repositories for tests (feature `test-support`).
//! Tests never use the network; fixture repos live in temp dirs.

use std::path::{Path, PathBuf};

use crate::{CommitId, Git};

/// A `Git` isolated from the machine's system and global Git config, with
/// its global config file placed under `scratch`.
pub fn isolated_git(scratch: &Path) -> Git {
    std::fs::create_dir_all(scratch).expect("create scratch dir");
    let global = scratch.join("gitconfig");
    if !global.exists() {
        std::fs::write(&global, "").expect("write empty gitconfig");
    }
    Git::new()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", global)
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        // Fixed dates make fixture commit SHAs reproducible.
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
}

/// A Git repository with a working tree, used as a source "remote".
#[derive(Debug, Clone)]
pub struct FixtureRepo {
    pub path: PathBuf,
    git: Git,
}

impl FixtureRepo {
    /// Creates an empty repo at `path` (branch `main`).
    pub fn init(path: impl Into<PathBuf>, git: Git) -> Self {
        let path = path.into();
        std::fs::create_dir_all(&path).expect("create fixture dir");
        git.run(Some(&path), ["init", "--quiet", "--initial-branch", "main"])
            .expect("git init");
        FixtureRepo { path, git }
    }

    /// Writes `contents` to `rel` (creating parent dirs).
    pub fn write(&self, rel: &str, contents: &str) -> &Self {
        let p = self.path.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).expect("create parent");
        std::fs::write(p, contents).expect("write fixture file");
        self
    }

    /// Removes a file or directory at `rel`.
    pub fn remove(&self, rel: &str) -> &Self {
        let p = self.path.join(rel);
        if p.is_dir() {
            std::fs::remove_dir_all(p).expect("remove fixture dir");
        } else {
            std::fs::remove_file(p).expect("remove fixture file");
        }
        self
    }

    /// Stages everything and commits; returns the new HEAD.
    pub fn commit(&self, message: &str) -> CommitId {
        self.git
            .run(Some(&self.path), ["add", "--all"])
            .expect("git add");
        self.git
            .run(
                Some(&self.path),
                [
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--quiet",
                    "--allow-empty",
                    "-m",
                    message,
                ],
            )
            .expect("git commit");
        self.git.head(&self.path).expect("rev-parse")
    }

    /// Like [`FixtureRepo::commit`], SSH-signed with the private key at
    /// `key`. Returns `None` if signing is unavailable (no `ssh-keygen`).
    pub fn commit_signed(&self, message: &str, key: &Path) -> Option<CommitId> {
        self.git
            .run(Some(&self.path), ["add", "--all"])
            .expect("git add");
        let signing_key = format!("user.signingkey={}", key.display());
        self.git
            .run(
                Some(&self.path),
                [
                    "-c",
                    "gpg.format=ssh",
                    "-c",
                    &signing_key,
                    "commit",
                    "--quiet",
                    "--allow-empty",
                    "-S",
                    "-m",
                    message,
                ],
            )
            .ok()?;
        Some(self.git.head(&self.path).expect("rev-parse"))
    }

    /// The repo path as a string URL accepted by `lo subscribe`.
    pub fn url(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

/// Generates an ed25519 key pair `<dir>/<name>` (+ `.pub`) with
/// `ssh-keygen`; returns the public key line, or `None` when `ssh-keygen`
/// isn't available.
pub fn ssh_keygen(dir: &Path, name: &str) -> Option<String> {
    std::fs::create_dir_all(dir).ok()?;
    let key = dir.join(name);
    let ok = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", name, "-f"])
        .arg(&key)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?
        .status
        .success();
    if !ok {
        return None;
    }
    let public = std::fs::read_to_string(dir.join(format!("{name}.pub"))).ok()?;
    let mut parts = public.split_whitespace();
    Some(format!("{} {}", parts.next()?, parts.next()?))
}

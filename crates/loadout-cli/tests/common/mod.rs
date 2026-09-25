//! Sandbox for end-to-end CLI tests: a fake home, config and data dir per
//! test, local fixture repos, and no access to the real user environment.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use loadout_git::Git;
use loadout_git::fixture::{FixtureRepo, isolated_git};

pub struct Sandbox {
    pub tmp: tempfile::TempDir,
    pub root: PathBuf,
    pub home: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub scratch: PathBuf,
    pub git: Git,
}

pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Sandbox {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        // Canonicalize so paths match what the binary sees (macOS /var -> /private/var).
        let root = dunce(tmp.path());
        let home = root.join("home");
        let config = root.join("config");
        let data = root.join("data");
        let scratch = root.join("scratch");
        std::fs::create_dir_all(&home).unwrap();
        let git = isolated_git(&scratch);
        Sandbox {
            tmp,
            root,
            home,
            config,
            data,
            scratch,
            git,
        }
    }

    /// A sandbox where Claude Code looks installed (`~/.claude` exists).
    pub fn with_claude() -> Self {
        let s = Self::new();
        std::fs::create_dir_all(s.home.join(".claude")).unwrap();
        s
    }

    pub fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("lo").unwrap();
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("LOADOUT_HOME", &self.home)
            .env("LOADOUT_CONFIG_DIR", &self.config)
            .env("LOADOUT_DATA_DIR", &self.data)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("XDG_DATA_HOME", self.home.join(".local/share"))
            .env("APPDATA", self.home.join("AppData/Roaming"))
            .env("LOCALAPPDATA", self.home.join("AppData/Local"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.scratch.join("gitconfig"))
            // Never touch the real OS keychain.
            .env("LOADOUT_TEST_KEYSTORE", self.keystore_file());
        // Windows needs these to spawn processes (git) at all.
        for var in [
            "SYSTEMROOT",
            "SystemRoot",
            "WINDIR",
            "TEMP",
            "TMP",
            "PATHEXT",
            "COMSPEC",
        ] {
            if let Some(v) = std::env::var_os(var) {
                cmd.env(var, v);
            }
        }
        cmd
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.run_env(args, &[])
    }

    /// Runs with extra environment variables.
    pub fn run_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut cmd = self.cmd();
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.args(args).output().unwrap();
        Output {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// Runs with `input` on stdin.
    pub fn run_stdin(&self, args: &[&str], input: &str) -> Output {
        let out = self.cmd().args(args).write_stdin(input).output().unwrap();
        Output {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// Runs and asserts exit code 0.
    pub fn ok(&self, args: &[&str]) -> Output {
        let out = self.run(args);
        assert_eq!(
            out.code, 0,
            "lo {args:?} failed\nstdout:\n{}\nstderr:\n{}",
            out.stdout, out.stderr
        );
        out
    }

    pub fn json(&self, args: &[&str]) -> serde_json::Value {
        let mut all = args.to_vec();
        all.push("--json");
        let out = self.ok(&all);
        serde_json::from_str(&out.stdout).unwrap()
    }

    /// A fixture source repo with a `LOADOUT.md` named `name`.
    pub fn source(&self, name: &str) -> FixtureRepo {
        let repo = FixtureRepo::init(self.root.join("remotes").join(name), self.git.clone());
        repo.write(
            "LOADOUT.md",
            &format!(
                "---\nloadout: 1\nname: {name}\nlayer: team\ngroup: payments-dev\n---\n# {name}\n"
            ),
        );
        repo
    }

    pub fn write_config(&self, text: &str) {
        std::fs::create_dir_all(&self.config).unwrap();
        std::fs::write(self.config.join("config.toml"), text).unwrap();
    }

    /// The file standing in for the OS keychain (`test-support` feature).
    pub fn keystore_file(&self) -> PathBuf {
        self.root.join("keystore.json")
    }

    pub fn skills_dir(&self) -> PathBuf {
        self.home.join(".claude").join("skills")
    }

    /// insta settings that replace sandbox paths and commit SHAs.
    pub fn insta(&self) -> insta::Settings {
        let mut s = insta::Settings::clone_current();
        let root = regex_escape(&self.root.to_string_lossy());
        s.add_filter(&root, "[ROOT]");
        s.add_filter(
            &regex_escape(&self.root.to_string_lossy().replace('\\', "\\\\")),
            "[ROOT]",
        );
        s.add_filter(r"\b[0-9a-f]{40}\b", "[SHA]");
        s.add_filter(r"\b[0-9a-f]{8}\b", "[SHORT-SHA]");
        s.add_filter(r"blake3:[0-9a-f]{64}", "blake3:[HASH]");
        s.add_filter(r"\\\\|\\", "/");
        s
    }
}

pub fn skill(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n\nDo the {name} thing.\n")
}

fn regex_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn dunce(p: &Path) -> PathBuf {
    let c = std::fs::canonicalize(p).unwrap();
    let s = c.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => c,
    }
}

const PREFIX: &str = "https://git.example.com/acme/";

pub fn copy_tree(from: &Path, repo: &FixtureRepo, rel: &str) {
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().into_owned();
        let child = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), repo, &child);
        } else {
            repo.write(&child, &std::fs::read_to_string(e.path()).unwrap());
        }
    }
}

/// Creates a local repo per `examples/` directory (under `remotes/`), points
/// the company config's `https://git.example.com/acme/<name>` URLs at them, and
/// returns the company config URL.
pub fn publish_examples(s: &Sandbox) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut repos: BTreeMap<String, FixtureRepo> = BTreeMap::new();
    for e in std::fs::read_dir(&root).unwrap() {
        let e = e.unwrap();
        if !e.file_type().unwrap().is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let repo = FixtureRepo::init(s.root.join("remotes").join(&name), s.git.clone());
        copy_tree(&e.path(), &repo, "");
        repos.insert(name, repo);
    }
    let company_config = &repos["acme-config"];
    let mut text = std::fs::read_to_string(company_config.path.join("LOADOUT.md")).unwrap();
    for (name, repo) in &repos {
        text = text.replace(&format!("\"{PREFIX}{name}\""), &format!("{:?}", repo.url()));
    }
    assert!(!text.contains(PREFIX), "every example URL maps to a repo");
    company_config.write("LOADOUT.md", &text);
    for repo in repos.values() {
        repo.commit("example");
    }
    company_config.url()
}

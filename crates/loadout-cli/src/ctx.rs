//! Shared command context: paths, config access, and output.

use std::fmt::Write as _;
use std::process::ExitCode;

use anyhow::{Context, Result};
use loadout_model::{Config, ConfigDoc};
use loadout_targets::fsutil::atomic_write;
use schemars::JsonSchema;
use serde::Serialize;

use crate::exit;
use crate::paths::Paths;

pub struct Ctx {
    pub paths: Paths,
    pub json: bool,
    pub quiet: bool,
}

impl Ctx {
    pub fn from_env(json: bool, quiet: bool) -> Result<Self> {
        Ok(Ctx {
            paths: Paths::from_env()?,
            json,
            quiet,
        })
    }

    /// Loads `config.toml` for editing; a missing file is an empty config.
    pub fn load_config_doc(&self) -> Result<ConfigDoc> {
        let path = self.paths.config_file();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        ConfigDoc::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    pub fn load_config(&self) -> Result<Config> {
        Ok(self.load_config_doc()?.config())
    }

    pub fn save_config(&self, doc: &ConfigDoc) -> Result<()> {
        let path = self.paths.config_file();
        atomic_write(&path, doc.to_string().as_bytes(), true)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Prints a command's result: JSON on stdout with `--json`, otherwise
    /// the human rendering (suppressed by `--quiet`), with warnings on
    /// stderr.
    pub fn emit<T: Report>(&self, report: &T) -> Result<()> {
        if self.json {
            println!("{}", serde_json::to_string_pretty(report)?);
            return Ok(());
        }
        for w in report.warnings() {
            eprintln!("warning: {w}");
        }
        if !self.quiet {
            let mut out = String::new();
            report.human(&mut out)?;
            print!("{out}");
        }
        Ok(())
    }
}

/// Picks the project layer when asked: `--project-dir <root>`, or
/// `--project` (the repo containing the current directory; for `init
/// --project`, the Git top-level or the current directory).
pub fn select_layer(
    ctx: Ctx,
    project: bool,
    project_dir: Option<&std::path::Path>,
    is_init: bool,
) -> Result<Ctx> {
    let root = match (project_dir, project) {
        (Some(dir), _) => Some(dir.to_path_buf()),
        (None, true) => {
            let cwd = std::env::current_dir()?;
            match Paths::find_project(&cwd) {
                Some(root) => Some(root),
                None if is_init => Some(
                    loadout_git::Git::new()
                        .run(Some(&cwd), ["rev-parse", "--show-toplevel"])
                        .ok()
                        .map(|s| std::path::PathBuf::from(s.trim()))
                        .unwrap_or(cwd),
                ),
                None => anyhow::bail!(
                    "not inside a project (no .loadout/config.toml above {}); run `lo init --project` in the repo",
                    cwd.display()
                ),
            }
        }
        (None, false) => None,
    };
    Ok(match root {
        Some(root) => Ctx {
            paths: ctx.paths.for_project(&root),
            ..ctx
        },
        None => ctx,
    })
}

impl Ctx {
    /// The project layer of the repo at `root`, with the same output
    /// settings.
    pub fn project(&self, root: &std::path::Path) -> Ctx {
        let user = Paths {
            project: None,
            ..self.paths.clone()
        };
        Ctx {
            paths: user.for_project(root),
            json: self.json,
            quiet: self.quiet,
        }
    }
}

/// A command result that renders both as JSON (via serde, schema in
/// `schemas/cli/`) and as human text.
pub trait Report: Serialize + JsonSchema {
    fn human(&self, out: &mut String) -> std::fmt::Result;

    fn warnings(&self) -> &[String] {
        &[]
    }
}

/// `--json` error output.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ErrorReport {
    pub error: String,
}

pub fn report_error(json: bool, e: &anyhow::Error) -> ExitCode {
    if json {
        let r = ErrorReport {
            error: format!("{e:#}"),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&r).unwrap_or_else(|_| "{}".into())
        );
    } else {
        let mut msg = format!("error: {e}");
        for cause in e.chain().skip(1) {
            let _ = write!(msg, "\n  caused by: {cause}");
        }
        eprintln!("{msg}");
    }
    ExitCode::from(exit::ERROR)
}

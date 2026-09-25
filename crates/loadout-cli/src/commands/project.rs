//! Project mode: `lo init --project` and the project half of
//! `lo sync`.

use std::fmt::Write as _;

use anyhow::{Result, bail};
use schemars::JsonSchema;
use serde::Serialize;

use crate::ctx::{Ctx, Report};
use crate::exit;
use crate::paths::PROJECT_DIR;

/// `--json` output of `lo init --project`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectInitReport {
    /// The repo root.
    pub root: String,
    /// The project's group (items rank as layer `project`).
    pub group: String,
    pub config: String,
}

impl Report for ProjectInitReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        writeln!(out, "Created {} for project {}.", self.config, self.group)?;
        writeln!(
            out,
            "Next: `lo --project subscribe <url>`, then `lo sync`; commit {PROJECT_DIR}/ so teammates get the same pins."
        )
    }
}

/// The project's group name: the repo directory, in kebab-case.
pub fn group_name(root: &std::path::Path) -> String {
    let base = root
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut out = String::new();
    for c in base.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() {
        "project".into()
    } else {
        out
    }
}

pub fn init(ctx: &Ctx) -> Result<u8> {
    let project = ctx.paths.project.as_ref().expect("project layer");
    let config = ctx.paths.config_file();
    if config.exists() {
        bail!("{} already exists", config.display());
    }
    let group = group_name(&project.root);
    let text = "# Loadout project layer. Commit this directory: teammates
# running `lo sync` in this repo get the same items, pinned by
# loadout.lock. Items from these sources rank as layer `project` and are
# installed into project paths (e.g. .claude/skills, .mcp.json).
#
# Add sources with `lo --project subscribe <url>`.
";
    loadout_targets::fsutil::atomic_write(&config, text.as_bytes(), false)?;
    ctx.emit(&ProjectInitReport {
        root: project.root.display().to_string(),
        group,
        config: config.display().to_string(),
    })?;
    Ok(exit::OK)
}

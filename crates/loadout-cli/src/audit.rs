//! Audit rules for this machine: built-ins plus the company config's
//! `audit_rules`.

use std::path::Path;

use anyhow::{Context, Result, bail};
use loadout_audit::{Finding, RuleSet};
use loadout_model::config::normalize_url;

use crate::ctx::Ctx;
use crate::membership::LoadedCompanyConfig;
use crate::scan::ScannedItem;

/// Built-in rules plus the company config's rule file(s): `audit_rules` is a path
/// inside the company config repo to a `.toml` file or a directory of them.
pub fn rules(ctx: &Ctx, company_config: Option<&LoadedCompanyConfig>) -> Result<RuleSet> {
    let mut set = RuleSet::builtin();
    let Some(c) = company_config else {
        return Ok(set);
    };
    let Some(rel) = &c.company_config.audit_rules else {
        return Ok(set);
    };
    if rel.starts_with('/') || rel.contains('\\') || rel.split('/').any(|p| p == "..") {
        bail!(
            "company config audit_rules must be a relative path inside the company config repo, got {rel:?}"
        );
    }
    let root = loadout_git::repo_dir(&ctx.paths.repos_dir(), &normalize_url(&c.url));
    let path = root.join(rel);
    let mut files: Vec<std::path::PathBuf> = if path.is_dir() {
        std::fs::read_dir(&path)
            .with_context(|| format!("reading company config audit rules {rel}"))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect()
    } else {
        vec![path]
    };
    files.sort();
    for f in files {
        let text = std::fs::read_to_string(&f).with_context(|| {
            format!("reading company config audit rules {}", display(&f, &root))
        })?;
        set.add_file(&display(&f, &root), &text)?;
    }
    Ok(set)
}

fn display(p: &Path, root: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Audits one scanned item.
pub fn audit(rules: &RuleSet, item: &ScannedItem) -> Vec<Finding> {
    let files: Vec<(&str, &[u8])> = item
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.contents.as_slice()))
        .collect();
    loadout_audit::audit_item(rules, &item.id, &files)
}

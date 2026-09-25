//! Review policy: which fetched changes apply now and which wait
//! in `state.json` for `lo approve`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use loadout_audit::{Finding, RuleSet, Severity};
use loadout_model::config::normalize_url;
use loadout_targets::fsutil::atomic_write;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::scan::ScannedSource;

pub const STATE_VERSION: u32 = 1;

/// Why a change was not applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    /// The source's layer is not in `auto_apply`.
    Review,
    /// Manual subscriptions are never auto-applied.
    ManualSource,
    /// The audit found `high` issues.
    Audit,
    /// The audit found `critical` issues; approving needs `--force-audit`.
    Blocked,
}

impl Reason {
    pub fn describe(self) -> &'static str {
        match self {
            Reason::Review => "needs review",
            Reason::ManualSource => "manual source",
            Reason::Audit => "audit findings",
            Reason::Blocked => "blocked by audit",
        }
    }
}

/// Item-level summary of a change (`kind/name` keys).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ItemChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
}

impl ItemChanges {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// A fetched change waiting for review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PendingChange {
    /// Source id from its `LOADOUT.md`.
    pub source: String,
    pub url: String,
    pub layer: String,
    /// Applied commit; absent for a source that isn't installed yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Fetched commit waiting for approval.
    pub to: String,
    pub reason: Reason,
    pub items: ItemChanges,
    /// Audit findings on the changed items.
    #[serde(default)]
    pub findings: Vec<FindingRecord>,
}

/// A stored audit finding (same fields as `lo audit --json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FindingRecord {
    pub rule: String,
    pub severity: Severity,
    pub message: String,
    pub item: String,
    pub file: String,
    pub line: usize,
    pub excerpt: String,
}

impl From<&Finding> for FindingRecord {
    fn from(f: &Finding) -> Self {
        FindingRecord {
            rule: f.rule.clone(),
            severity: f.severity,
            message: f.message.clone(),
            item: f.item.to_string(),
            file: f.file.clone(),
            line: f.line,
            excerpt: f.excerpt.clone(),
        }
    }
}

/// `state.json`: last successful sync and pending changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<String>,
    #[serde(default)]
    pub pending: Vec<PendingChange>,
}

impl Default for State {
    fn default() -> Self {
        State {
            version: STATE_VERSION,
            last_sync: None,
            pending: Vec::new(),
        }
    }
}

impl State {
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let s: State = serde_json::from_str(&text)
            .with_context(|| format!("{} is corrupt; move it aside", path.display()))?;
        if s.version != STATE_VERSION {
            bail!("{} has unsupported version {}", path.display(), s.version);
        }
        Ok(s)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut json = serde_json::to_vec_pretty(self)?;
        json.push(b'\n');
        atomic_write(path, &json, false).with_context(|| format!("writing {}", path.display()))
    }

    pub fn pending_for(&self, url: &str) -> Option<&PendingChange> {
        let url = normalize_url(url);
        self.pending.iter().find(|p| normalize_url(&p.url) == url)
    }
}

/// What to do with a fetched change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Apply,
    Hold(Reason),
}

/// Decides per change.
pub struct Reviewer<'a> {
    pub rules: &'a RuleSet,
    /// `company.policy.auto_apply` layers.
    pub auto_apply_layers: &'a [String],
    /// `[policy] auto_apply` in config.toml: source names or URLs.
    pub local_auto_apply: &'a [String],
    /// `sync --yes`.
    pub approve_all: bool,
    /// `--force-audit`.
    pub force_audit: bool,
}

/// One fetched change to decide on.
pub struct Change<'a> {
    pub url: &'a str,
    pub manual: bool,
    pub old: Option<&'a ScannedSource>,
    pub new: &'a ScannedSource,
}

impl Reviewer<'_> {
    pub fn decide(&self, c: &Change<'_>) -> (Verdict, Vec<Finding>) {
        let findings = audit_changed(self.rules, c.old, c.new);
        let worst = loadout_audit::worst(&findings);
        let local = self
            .local_auto_apply
            .iter()
            .any(|a| a == &c.new.manifest.name || normalize_url(a) == normalize_url(c.url));
        let verdict = if worst == Some(Severity::Critical) && !self.force_audit {
            Verdict::Hold(Reason::Blocked)
        } else if self.approve_all {
            Verdict::Apply
        } else if worst >= Some(Severity::High) {
            Verdict::Hold(Reason::Audit)
        } else if c.old.is_none() || local {
            // Joining a group or subscribing is the approval of a new source.
            Verdict::Apply
        } else if c.manual {
            Verdict::Hold(Reason::ManualSource)
        } else if self.auto_apply_layers.contains(&c.new.default_layer) {
            Verdict::Apply
        } else {
            Verdict::Hold(Reason::Review)
        };
        (verdict, findings)
    }
}

/// Audits the items that are new or whose content changed.
pub fn audit_changed(
    rules: &RuleSet,
    old: Option<&ScannedSource>,
    new: &ScannedSource,
) -> Vec<Finding> {
    let before: BTreeMap<String, &str> = old
        .map(|o| {
            o.items
                .iter()
                .map(|i| (i.id.key().to_string(), i.content_hash.as_str()))
                .collect()
        })
        .unwrap_or_default();
    new.items
        .iter()
        .filter(|i| before.get(&i.id.key().to_string()) != Some(&i.content_hash.as_str()))
        .flat_map(|i| crate::audit::audit(rules, i))
        .collect()
}

/// Items added, removed and changed between two scans of a source.
pub fn item_changes(old: Option<&ScannedSource>, new: &ScannedSource) -> ItemChanges {
    let before: BTreeMap<String, &str> = old
        .map(|o| {
            o.items
                .iter()
                .map(|i| (i.id.key().to_string(), i.content_hash.as_str()))
                .collect()
        })
        .unwrap_or_default();
    let after: BTreeMap<String, &str> = new
        .items
        .iter()
        .map(|i| (i.id.key().to_string(), i.content_hash.as_str()))
        .collect();
    let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let mut out = ItemChanges::default();
    for k in keys {
        match (before.get(k), after.get(k)) {
            (None, Some(_)) => out.added.push(k.clone()),
            (Some(_), None) => out.removed.push(k.clone()),
            (Some(a), Some(b)) if a != b => out.changed.push(k.clone()),
            _ => {}
        }
    }
    out
}

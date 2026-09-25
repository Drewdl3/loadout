//! The apply pipeline shared by `sync`, `enable`, `disable` and `prefer`:
//! sources → candidates → resolution → store → targets.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use loadout_core::links::{self, Action, Desired};
use loadout_core::resolve::{self, Candidate, Resolution};
use loadout_git::Git;
use loadout_model::config::normalize_url;
use loadout_model::{
    Config, Groups, ItemId, ItemKind, LayerModel, LinkMode, Lock, LockedItem, LockedSource,
    Profile, SourceSub,
};
use loadout_targets::fsutil::atomic_write;
use loadout_targets::owned::OwnedFile;
use loadout_targets::table::Layout;
use loadout_targets::testhook;
use loadout_targets::{TargetDef, TargetTable, expand_path, link};
use schemars::JsonSchema;
use serde::Serialize;

use crate::ctx::Ctx;
use crate::membership::{self, LoadedCompanyConfig};
use crate::paths::Paths;
use crate::review::{FindingRecord, PendingChange, Reason, Reviewer, State};
use crate::scan::{ScannedItem, item_file_name};
pub use crate::sources::Fetch;
use crate::sources::{self, Fetched, LoadOptions, load_lock, source_list};
use crate::state::{Placement, Resolved, ResolvedItem, ResolvedSource};
use crate::store;
use crate::{mcp, secrets, signing};

/// A target's project-relative path inside the repo at `root`; `None` if
/// it is absolute or escapes the repo.
pub fn project_path(root: &Path, rel: &str) -> Option<std::path::PathBuf> {
    let bad = rel.is_empty()
        || rel.starts_with('/')
        || rel.starts_with('~')
        || rel.contains('\\')
        || rel.contains(':')
        || rel.split('/').any(|c| c == "..");
    (!bad).then(|| {
        rel.split('/')
            .filter(|c| !c.is_empty())
            .fold(root.to_path_buf(), |p, c| p.join(c))
    })
}

/// Seconds since the Unix epoch.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Absolute path of the running `lo` binary, written into target
/// configs for `mcp-run` / `mcp-headers`.
pub fn loadout_bin() -> String {
    std::env::current_exe().map_or_else(|_| "lo".to_owned(), |p| loadout_bin_for(&p))
}

/// How the binary at `exe` is written into target configs.
pub fn loadout_bin_for(exe: &Path) -> String {
    dunce_canonical(exe)
        .unwrap_or_else(|| exe.to_path_buf())
        .display()
        .to_string()
}

/// Canonicalizes without Windows' `\\?\` prefix.
fn dunce_canonical(p: &Path) -> Option<std::path::PathBuf> {
    let c = std::fs::canonicalize(p).ok()?;
    let s = c.to_string_lossy();
    Some(match s.strip_prefix(r"\\?\") {
        Some(rest) => std::path::PathBuf::from(rest),
        None => c,
    })
}

/// Whether to re-run membership discovery first (company config only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    Never,
    /// When `profile_refresh_interval` has passed (scheduled syncs).
    IfDue,
    Always,
}

pub struct Options {
    pub fetch: Fetch,
    /// The current time (seconds since the Unix epoch), for `fetched_at`.
    pub now: i64,
    pub refresh: Refresh,
    /// A company config the caller already loaded (avoids fetching it twice).
    pub company_config: Option<LoadedCompanyConfig>,
    /// `sync --yes`: apply held changes too (except audit-blocked ones).
    pub approve_all: bool,
    /// `--force-audit`: critical findings don't block.
    pub force_audit: bool,
    /// `lo approve`: normalized URL → commit to install.
    pub approved: BTreeMap<String, String>,
    /// `sync --dry-run`: plan only, change nothing.
    pub dry_run: bool,
}

impl Options {
    pub fn new(fetch: Fetch) -> Self {
        Options {
            fetch,
            now: unix_now(),
            refresh: Refresh::Never,
            company_config: None,
            approve_all: false,
            force_audit: false,
            approved: BTreeMap::new(),
            dry_run: false,
        }
    }
}

/// What an apply did. `--json` output of `sync`, `enable`, `disable`,
/// `prefer`.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct ApplyReport {
    pub sources: Vec<SourceReport>,
    /// Number of enabled items (target-independent view).
    pub enabled_items: usize,
    pub targets: Vec<TargetReport>,
    /// True when some `kind/name` had an unresolved equal-rank conflict.
    pub conflicts: bool,
    /// True when membership discovery ran and `[profile]` was updated.
    pub profile_refreshed: bool,
    /// Changes waiting for `lo approve` (after this run).
    pub pending: Vec<PendingChange>,
    /// True when a pending change is blocked by a critical audit finding.
    pub audit_blocked: bool,
    /// Audit findings on changes that were applied.
    pub findings: Vec<FindingRecord>,
    /// `--dry-run`: nothing was changed; targets show what would change.
    pub dry_run: bool,
    /// `--if-stale`: the last sync is recent, nothing was done.
    pub up_to_date: bool,
    /// Updates refused because their commits aren't signed by an allowed
    /// signer; the previous pin is kept.
    pub signature_failures: Vec<String>,
    /// `sync` inside a repo with a project layer: that layer's result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<Box<ApplyReport>>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SourceReport {
    pub name: String,
    pub url: String,
    pub commit: String,
    /// The previously applied commit, when this run moved the source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_from: Option<String>,
    pub items: usize,
    /// True when the checkout was not fetched (cached mode or fetch failure).
    pub cached: bool,
    /// The source whose `upstream:` pulled this one in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct TargetReport {
    pub id: String,
    /// Item keys (`kind/name`) newly installed.
    pub added: Vec<String>,
    /// Item keys whose entry was replaced.
    pub updated: Vec<String>,
    /// Item keys removed.
    pub removed: Vec<String>,
    pub unchanged: usize,
    /// Paths skipped because something Loadout doesn't own is there.
    pub collisions: Vec<PathReport>,
    pub errors: Vec<PathReport>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PathReport {
    pub item: String,
    pub path: String,
    pub message: String,
}

impl ApplyReport {
    pub fn has_errors(&self) -> bool {
        self.targets.iter().any(|t| !t.errors.is_empty())
    }
}

/// Runs the whole pipeline: load (and review) sources, resolve, rebuild the
/// store, reconcile targets, and persist `owned.json`, `resolved.json`,
/// `loadout.lock` and `state.json`. With `dry_run`, only plans.
pub fn apply(ctx: &Ctx, config: &Config, opts: Options) -> Result<ApplyReport> {
    let mut report = ApplyReport {
        dry_run: opts.dry_run,
        ..ApplyReport::default()
    };
    let git = Git::new();
    let fetch = opts.fetch;
    let mut config = config.clone();
    if ctx.paths.project.is_some() {
        // A project layer has its own sources only.
        config.company_config = None;
    }
    let lock = load_lock(&ctx.paths)?;
    let mut state = State::load(&ctx.paths.state_file())?;
    let approved = &opts.approved;
    let mut company_config_fetched: Option<Fetched> = None;
    let mut pending: Vec<PendingChange> = Vec::new();
    let mut findings: Vec<loadout_audit::Finding> = Vec::new();

    // The company config first: its policy decides everything else.
    let company_config = match (opts.company_config, config.company_config.clone()) {
        (Some(c), _) => Some(c),
        (None, None) => None,
        (None, Some(url)) => {
            let dir = sources::repo_dir(&ctx.paths, &url);
            match fetch {
                Fetch::Locked => {
                    let pin = lock.source(&url).with_context(|| {
                        format!(
                            "loadout.lock has no entry for the company config {url}; run `lo sync` first"
                        )
                    })?;
                    sources::pin_checkout(&git, &ctx.paths, &url, &pin.commit)?;
                }
                Fetch::Cached => {
                    let commit = approved
                        .get(&normalize_url(&url))
                        .cloned()
                        .or_else(|| sources::applied_commit(&git, &ctx.paths, &lock, &url));
                    if let Some(c) = commit {
                        sources::pin_checkout(&git, &ctx.paths, &url, &c)?;
                    }
                }
                Fetch::Remote if dir.join(".git").exists() => {
                    let old = membership::load_company_config(ctx, &git, &url, false).ok();
                    let rules = crate::audit::rules(ctx, old.as_ref())?;
                    let old_signing = signing::prepare(old.as_ref().map(|c| &c.company_config))?;
                    let reviewer = Reviewer {
                        rules: &rules,
                        auto_apply_layers: old
                            .as_ref()
                            .map_or(&[][..], |c| &c.company_config.policy.auto_apply),
                        local_auto_apply: &config.policy.auto_apply,
                        approve_all: opts.approve_all,
                        force_audit: opts.force_audit,
                    };
                    let sub = SourceSub {
                        layer: Some("company".into()),
                        group: old.as_ref().map(|c| c.company_config.name.clone()),
                        ..SourceSub::new(url.clone())
                    };
                    let one = sources::load_one(
                        &git,
                        &ctx.paths,
                        &sub,
                        false,
                        &LoadOptions {
                            fetch,
                            lock: &lock,
                            reviewer: Some(&reviewer),
                            approved: &BTreeMap::new(),
                            dry_run: opts.dry_run,
                            signing: old_signing.as_ref(),
                        },
                    )
                    .with_context(|| format!("company config {url}"))?;
                    report.warnings.extend(one.warning);
                    report.signature_failures.extend(one.signature_error);
                    pending.extend(one.pending);
                    findings.extend(one.findings);
                    match one.fetched {
                        Some(f) => company_config_fetched = Some(f),
                        None => bail!(
                            "the company config {url} was not installed (held for review or refused); see `lo status`"
                        ),
                    }
                }
                Fetch::Remote => {}
            }
            let first_fetch = fetch == Fetch::Remote && !dir.join(".git").exists();
            Some(membership::load_company_config(
                ctx,
                &git,
                &url,
                first_fetch,
            )?)
        }
    };
    if let Some(c) = &company_config {
        report.warnings.extend(c.warning.clone());
        let due = match opts.refresh {
            Refresh::Never => false,
            Refresh::Always => true,
            Refresh::IfDue => membership::refresh_due(ctx, &config, &c.url)?,
        };
        if due && !opts.dry_run {
            let d = membership::run_discovery(&c.company_config, &config);
            membership::save_discovery(ctx, &c.url, &d)?;
            report.warnings.extend(d.warnings);
            config.profile = loadout_members::profile_table(&d.profile)
                .into_iter()
                .map(|(k, v)| (k, Groups(v)))
                .collect();
            report.profile_refreshed = true;
        }
    }
    let config = &config;
    let rules = crate::audit::rules(ctx, company_config.as_ref())?;
    let reviewer = Reviewer {
        rules: &rules,
        auto_apply_layers: company_config
            .as_ref()
            .map_or(&[][..], |c| &c.company_config.policy.auto_apply),
        local_auto_apply: &config.policy.auto_apply,
        approve_all: opts.approve_all,
        force_audit: opts.force_audit,
    };
    let signing = signing::prepare(company_config.as_ref().map(|c| &c.company_config))?;
    if let Some(p) = signing.as_ref().and_then(|s| s.problem.clone()) {
        report.warnings.push(p);
    }
    let mut subs = source_list(config, company_config.as_ref(), &mut report.warnings);
    // The company config was reviewed above; use that result.
    if let Some(f) = &company_config_fetched {
        let url = normalize_url(&f.sub.url);
        subs.retain(|p| normalize_url(&p.sub.url) != url);
    }
    let load_opts = LoadOptions {
        fetch,
        lock: &lock,
        reviewer: Some(&reviewer),
        approved,
        dry_run: opts.dry_run,
        signing: signing.as_ref(),
    };
    let mut loaded = sources::load_sources(&git, &ctx.paths, &subs, &load_opts)?;
    if let Some(mut f) = company_config_fetched {
        if let Some(c) = &company_config {
            f.sub.group = Some(c.company_config.name.clone());
        }
        loaded.sources.push(f);
    }
    // Sources pulled in by the `upstream:` of what's loaded (and theirs).
    let mut seen: BTreeSet<String> = subs.iter().map(|p| normalize_url(&p.sub.url)).collect();
    seen.extend(loaded.sources.iter().map(|f| normalize_url(&f.sub.url)));
    sources::load_upstreams(
        &git,
        &ctx.paths,
        &mut loaded,
        &mut seen,
        company_config.as_ref(),
        &load_opts,
    );
    report
        .signature_failures
        .append(&mut loaded.signature_errors);
    report.warnings.append(&mut loaded.warnings);
    pending.append(&mut loaded.pending);
    findings.append(&mut loaded.findings);
    let fetched = dedupe_sources(loaded.sources, &mut report);
    let items: BTreeMap<&ItemId, &ScannedItem> = fetched
        .iter()
        .flat_map(|f| f.scanned.items.iter().map(|i| (&i.id, i)))
        .collect();
    let candidates: Vec<Candidate> = fetched
        .iter()
        .flat_map(|f| {
            f.scanned
                .items
                .iter()
                .map(move |i| candidate(i, f.sub.priority))
        })
        .collect();

    let layers = company_config
        .as_ref()
        .map_or_else(LayerModel::default, |c| c.company_config.layer_model());
    let mut profile = effective_profile(config, &fetched);
    if let Some(c) = &company_config {
        profile.add("company", c.company_config.name.as_str());
    }
    let run = |target: Option<&str>| {
        resolve::resolve(&resolve::Input {
            layers: &layers,
            profile: &profile,
            candidates: &candidates,
            toggles: &config.toggles,
            prefer: &config.prefer,
            target,
        })
    };

    let overall = run(None);
    let table = TargetTable::load(&ctx.paths.user_targets_dir())?;
    let (targets, target_warnings) = enabled_targets(&table, config, &ctx.paths.home);
    let per_target: Vec<(&TargetDef, Resolution)> =
        targets.iter().map(|t| (*t, run(Some(&t.id)))).collect();

    let mut warnings: BTreeSet<String> = BTreeSet::new();
    for r in std::iter::once(&overall).chain(per_target.iter().map(|(_, r)| r)) {
        warnings.extend(r.warnings.iter().map(ToString::to_string));
    }
    report.warnings.extend(target_warnings);
    report.warnings.extend(warnings);
    report.conflicts = overall.has_conflicts() || per_target.iter().any(|(_, r)| r.has_conflicts());

    // Store: every winner of any resolution.
    let winners: BTreeSet<&ItemId> = std::iter::once(&overall)
        .chain(per_target.iter().map(|(_, r)| r))
        .flat_map(|r| r.items.iter().filter_map(|i| i.winner.as_ref()))
        .collect();
    if fetch == Fetch::Locked {
        verify_locked(&lock, &winners, &items)?;
    }
    if !opts.dry_run {
        store::rebuild(
            &ctx.paths.store_dir(),
            winners.iter().map(|id| (*id, items[id].files.as_slice())),
        )?;
        testhook::crash_point("after-store");
    }

    let source_dirs: BTreeMap<String, std::path::PathBuf> = fetched
        .iter()
        .map(|f| {
            (
                f.scanned.manifest.name.clone(),
                loadout_git::repo_dir(&ctx.paths.repos_dir(), &normalize_url(&f.sub.url)),
            )
        })
        .collect();
    let loadout_bin = loadout_bin();
    let system_cell: std::cell::OnceCell<secrets::System> = std::cell::OnceCell::new();
    let chain = || secrets::chain(config, company_config.as_ref().map(|c| &c.company_config));
    let system = || -> Result<&secrets::System> {
        if let Some(s) = system_cell.get() {
            return Ok(s);
        }
        let s = secrets::System::new(chain()?);
        Ok(system_cell.get_or_init(|| s))
    };
    let project_root = ctx.paths.project.as_ref().map(|p| p.root.clone());
    let loadout_args: Vec<String> = match &project_root {
        Some(root) => vec!["--project-dir".into(), root.display().to_string()],
        None => Vec::new(),
    };
    let mcp_env = mcp::McpEnv {
        loadout_args: &loadout_args,
        project_root: project_root.as_deref(),
        home: &ctx.paths.home,
        items: &items,
        source_dirs: &source_dirs,
        loadout_bin: &loadout_bin,
        allow_secrets_on_disk: config.secrets.allow_secrets_on_disk,
        system: &system,
        dry_run: opts.dry_run,
    };
    let mut owned = OwnedFile::load(&ctx.paths.owned_file())?;
    // Targets no longer enabled: remove what we own there.
    let stale: Vec<String> = owned
        .targets
        .keys()
        .chain(owned.mcp.keys())
        .chain(owned.plugins.keys())
        .filter(|id| !targets.iter().any(|t| &t.id == *id))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    // Plan every target first, then record what we are about to own
    // (crash safety): if the process dies while placing
    // entries, the next sync still knows they are ours.
    let plans: Vec<TargetPlan<'_>> = per_target
        .iter()
        .map(|(target, resolution)| {
            let desired = desired_entries(&ctx.paths, config, target, resolution, &items);
            let before = owned.entries(&target.id).to_vec();
            let plan = links::plan(&desired, &before, link::inspect);
            TargetPlan {
                target,
                resolution,
                desired,
                before,
                plan,
            }
        })
        .collect();
    if !opts.dry_run {
        let mut intent = owned.clone();
        let plan_env = mcp::McpEnv {
            dry_run: true,
            ..mcp_env.clone()
        };
        for TargetPlan {
            target,
            resolution,
            before,
            plan,
            ..
        } in &plans
        {
            intent.set(&target.id, link::intent(before, plan));
            let mut scratch = owned.clone();
            mcp::reconcile_target(
                &plan_env,
                target,
                resolution,
                &mut scratch,
                &mut TargetReport::default(),
                &mut Vec::new(),
            );
            if let Some(planned) = scratch.mcp.get(&target.id) {
                let entry = intent
                    .mcp
                    .entry(target.id.clone())
                    .or_insert_with(|| planned.clone());
                entry.servers.extend(planned.servers.iter().cloned());
            }
        }
        intent
            .save(&ctx.paths.owned_file())
            .context("writing owned.json")?;
        testhook::crash_point("after-intent");
    }

    let mut placements: BTreeMap<String, Vec<Placement>> = BTreeMap::new();
    let mut no_plugins: BTreeSet<String> = BTreeSet::new();
    let mut plugin_count = 0;
    for TargetPlan {
        target,
        resolution,
        desired,
        before,
        plan,
    } in &plans
    {
        let applied = if opts.dry_run {
            link::Applied::default()
        } else {
            link::apply(plan)
        };
        let mut t = summarize(&target.id, plan, &applied, &ctx.paths.home, before);
        mcp::reconcile_target(
            &mcp_env,
            target,
            resolution,
            &mut owned,
            &mut t,
            &mut report.warnings,
        );
        let takes_plugins = crate::plugins::reconcile_target(
            &ctx.paths,
            target,
            resolution,
            &items,
            &mut owned,
            &mut t,
            &mut report.warnings,
            opts.dry_run,
        );
        let n = crate::plugins::enabled_plugins(resolution).len();
        if !takes_plugins && n > 0 {
            no_plugins.insert(target.id.clone());
            plugin_count = plugin_count.max(n);
        }
        testhook::crash_point("target");
        for c in t.collisions.iter().filter(|c| !c.item.starts_with("mcp/")) {
            report.warnings.push(format!(
                "{}: {} already exists and is not managed by Loadout; skipped {}",
                target.id, c.path, c.item
            ));
        }
        placements.insert(
            target.id.clone(),
            desired
                .iter()
                .map(|d| Placement {
                    id: winner_of(resolution, d),
                    dest: d.dest.clone(),
                    mode: d.mode,
                    installed: applied.owned.iter().any(|o| o.dest == d.dest),
                })
                .collect(),
        );
        owned.set(&target.id, applied.owned);
        report.targets.push(t);
    }
    report.warnings.extend(crate::plugins::unsupported_note(
        &no_plugins,
        plugin_count,
        ctx.paths.project.is_some(),
    ));
    for id in stale {
        let plan = links::plan(&[], owned.entries(&id), link::inspect);
        let applied = if opts.dry_run {
            link::Applied::default()
        } else {
            link::apply(&plan)
        };
        let mut t = summarize(&id, &plan, &applied, &ctx.paths.home, &[]);
        mcp::remove_all(
            &ctx.paths.home,
            &id,
            &mut owned,
            &mut t,
            &mut report.warnings,
            opts.dry_run,
        );
        crate::plugins::remove_all(
            &ctx.paths.home,
            &id,
            &mut owned,
            &mut t,
            &mut report.warnings,
            opts.dry_run,
        );
        report.targets.push(t);
        owned.set(&id, applied.owned);
    }
    // Held changes: fresh from a remote sync, otherwise the previous ones
    // minus what was just approved.
    let pending = match fetch {
        Fetch::Remote => pending,
        Fetch::Cached | Fetch::Locked => state
            .pending
            .iter()
            .filter(|p| approved.get(&normalize_url(&p.url)) != Some(&p.to))
            .cloned()
            .collect(),
    };
    report.audit_blocked = pending.iter().any(|p| p.reason == Reason::Blocked);
    report.findings = findings.iter().map(FindingRecord::from).collect();
    for p in &pending {
        report.warnings.push(format!(
            "{}: update {} → {} held ({}); see `lo diff`, then `lo approve {}`",
            p.source,
            p.from.as_deref().map_or("(new)", sources::short),
            sources::short(&p.to),
            p.reason.describe(),
            p.source
        ));
    }
    report.pending = pending;
    if opts.dry_run {
        report.enabled_items = overall.items.iter().filter(|i| i.enabled).count();
        return Ok(report);
    }
    testhook::crash_point("before-owned");
    owned
        .save(&ctx.paths.owned_file())
        .context("writing owned.json")?;

    let docs: Vec<loadout_search::Doc> = fetched
        .iter()
        .flat_map(|f| &f.scanned.items)
        .map(|i| crate::search::doc(i, crate::search::state_of(i, &overall)))
        .collect();
    let now = loadout_model::time::format_rfc3339(opts.now);
    let new = new_lock(&lock, &fetched, &winners, &items, &now);
    let resolved = Resolved {
        sources: fetched
            .iter()
            .map(|f| ResolvedSource {
                name: f.scanned.manifest.name.clone(),
                url: f.sub.url.clone(),
                commit: f.commit.clone(),
                via: f.via.clone(),
                upstream: f
                    .scanned
                    .manifest
                    .upstream
                    .iter()
                    .map(|u| u.resolve(&f.sub.url))
                    .collect(),
                layer: f
                    .sub
                    .layer
                    .clone()
                    .unwrap_or_else(|| f.scanned.manifest.layer.clone()),
                group: f
                    .sub
                    .group
                    .clone()
                    .or_else(|| f.scanned.manifest.group.clone()),
            })
            .collect(),
        items: overall
            .items
            .iter()
            .filter_map(|r| {
                let id = r.winner.as_ref()?;
                Some(list_item(items[id], r))
            })
            .collect(),
        resolution: overall,
        placements,
        ..Resolved::default()
    };
    testhook::crash_point("before-resolved");
    resolved.save(&ctx.paths.resolved_file())?;
    crate::search::save(&ctx.paths.search_file(), &docs)?;
    testhook::crash_point("before-lock");
    if new != lock {
        atomic_write(&ctx.paths.lock_file(), new.to_toml().as_bytes(), false)
            .context("writing loadout.lock")?;
    }
    testhook::crash_point("before-state");
    state.pending = report.pending.clone();
    if fetch != Fetch::Cached {
        state.last_sync = Some(now);
    }
    state.save(&ctx.paths.state_file())?;
    report.enabled_items = resolved.items.iter().filter(|i| i.enabled).count();
    Ok(report)
}

/// One enabled target's link plan, computed before anything is placed.
struct TargetPlan<'a> {
    target: &'a TargetDef,
    resolution: &'a Resolution,
    desired: Vec<Desired>,
    before: Vec<links::Owned>,
    plan: links::Plan,
}

fn winner_of(resolution: &Resolution, d: &Desired) -> ItemId {
    resolution
        .get(&d.item)
        .and_then(|r| r.winner.clone())
        .expect("desired entries come from winners")
}

fn list_item(item: &ScannedItem, r: &resolve::Resolved) -> ResolvedItem {
    let key = item.id.key();
    ResolvedItem {
        id: item.id.clone(),
        kind: key.kind(),
        name: key.name().to_owned(),
        source: item.id.source().to_owned(),
        description: item.meta.description.clone(),
        layer: item.loadout.layer.clone().unwrap_or_default(),
        group: item.loadout.group.clone(),
        enabled: r.enabled,
        content_hash: item.content_hash.clone(),
        targets: item.loadout.targets.clone(),
        author: item.loadout.author.clone(),
        co_authors: item.loadout.co_authors.clone(),
    }
}

fn candidate(item: &ScannedItem, priority: i64) -> Candidate {
    let g = &item.loadout;
    Candidate {
        id: item.id.clone(),
        layer: g.layer.clone().unwrap_or_default(),
        group: g.group.clone(),
        applies_to: g.applies_to.clone(),
        mode: g.mode,
        locked: g.locked.unwrap_or(false),
        overridable: g.overridable.unwrap_or(true),
        targets: g.targets.clone(),
        priority,
        content_hash: item.content_hash.clone(),
    }
}

/// `[profile]` from config, plus the layer/group of each manual source and
/// each source pulled in through `upstream:`: subscribing to a source makes
/// you a member of its group, and so does a source you
/// subscribed to naming it as its upstream.
fn effective_profile(config: &Config, fetched: &[Fetched]) -> Profile {
    let mut p = membership::cached_profile(config);
    for f in fetched.iter().filter(|f| f.manual || f.via.is_some()) {
        if let Some(group) = &f.scanned.default_group {
            p.add(f.scanned.default_layer.as_str(), group.as_str());
        }
    }
    p
}

/// The lock for what was just applied. `fetched_at` is kept from the old
/// lock while a source's commit is unchanged.
fn new_lock(
    old: &Lock,
    fetched: &[Fetched],
    winners: &BTreeSet<&ItemId>,
    items: &BTreeMap<&ItemId, &ScannedItem>,
    now: &str,
) -> Lock {
    Lock {
        sources: fetched
            .iter()
            .map(|f| LockedSource {
                url: f.sub.url.clone(),
                name: f.scanned.manifest.name.clone(),
                commit: f.commit.clone(),
                fetched_at: old
                    .source(&f.sub.url)
                    .filter(|s| s.commit == f.commit)
                    .map_or_else(|| now.to_owned(), |s| s.fetched_at.clone()),
            })
            .collect(),
        items: winners
            .iter()
            .map(|id| LockedItem {
                id: id.to_string(),
                content_hash: items[id].content_hash.clone(),
            })
            .collect(),
        ..Lock::default()
    }
}

/// `--locked`: every winner must be pinned with the same content hash.
fn verify_locked(
    lock: &Lock,
    winners: &BTreeSet<&ItemId>,
    items: &BTreeMap<&ItemId, &ScannedItem>,
) -> Result<()> {
    let mut problems = Vec::new();
    for id in winners {
        match lock.item(&id.to_string()) {
            None => problems.push(format!("{id} is not in loadout.lock")),
            Some(l) if l.content_hash != items[id].content_hash => problems.push(format!(
                "{id} content differs from loadout.lock ({} vs {})",
                items[id].content_hash, l.content_hash
            )),
            Some(_) => {}
        }
    }
    if !problems.is_empty() {
        bail!(
            "--locked: the lockfile does not match; nothing was changed:\n  {}",
            problems.join("\n  ")
        );
    }
    Ok(())
}

/// Two subscriptions whose `LOADOUT.md` share a name would produce colliding
/// item ids; keep the first (by URL) and warn.
fn dedupe_sources(fetched: Vec<Fetched>, report: &mut ApplyReport) -> Vec<Fetched> {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let mut out = Vec::new();
    for f in fetched {
        let name = f.scanned.manifest.name.clone();
        if let Some(first) = seen.get(&name) {
            report.warnings.push(format!(
                "sources {first} and {} both call themselves {name:?}; ignoring the latter",
                f.sub.url
            ));
            continue;
        }
        seen.insert(name.clone(), f.sub.url.clone());
        report.sources.push(SourceReport {
            name,
            url: f.sub.url.clone(),
            commit: f.commit.clone(),
            updated_from: f.updated_from.clone(),
            items: f.scanned.items.len(),
            cached: f.cached,
            via: f.via.clone(),
        });
        out.push(f);
    }
    out
}

/// `[targets] enabled`, or every known target whose `detect` path exists.
/// Returns the targets plus warnings about unknown or undetected targets.
pub fn enabled_targets<'t>(
    table: &'t TargetTable,
    config: &Config,
    home: &Path,
) -> (Vec<&'t TargetDef>, Vec<String>) {
    let mut warnings = Vec::new();
    let targets = match &config.targets.enabled {
        Some(ids) => ids
            .iter()
            .filter_map(|id| {
                let t = table.get(id);
                if t.is_none() {
                    warnings.push(format!(
                        "unknown target {id:?} in [targets] enabled; skipped"
                    ));
                }
                t
            })
            .collect(),
        None => {
            let found: Vec<&TargetDef> = table
                .iter()
                .filter(|t| {
                    t.detect
                        .iter()
                        .filter_map(|d| expand_path(d, home))
                        .any(|p| p.exists())
                })
                .collect();
            if found.is_empty() {
                warnings.push(
                    "no AI tools detected; set `[targets] enabled = [\"claude-code\"]` in config.toml"
                        .into(),
                );
            }
            found
        }
    };
    (targets, warnings)
}

/// The entries `target` should contain: its enabled winners.
fn desired_entries(
    paths: &Paths,
    config: &Config,
    target: &TargetDef,
    resolution: &Resolution,
    items: &BTreeMap<&ItemId, &ScannedItem>,
) -> Vec<Desired> {
    let mode = config.targets.link_mode_for(&target.id);
    let store = paths.store_dir();
    let mut out = Vec::new();
    for r in resolution.items.iter().filter(|r| r.enabled) {
        let Some(id) = &r.winner else { continue };
        let key = id.key();
        // Where the item goes, and whether it is a directory or one file.
        let (path, project, layout) = match key.kind() {
            ItemKind::Extra => {
                let Some((ty, _)) = key.name().split_once('.') else {
                    continue;
                };
                let Some(spec) = target.extras.get(ty) else {
                    continue;
                };
                (&spec.path, &spec.project, Layout::FilePerItem)
            }
            kind => match target.dir_for(kind) {
                Some(spec) => (&spec.path, &spec.project, spec.layout),
                None => continue,
            },
        };
        let dir = match &paths.project {
            Some(p) => match project
                .as_deref()
                .and_then(|rel| project_path(&p.root, rel))
            {
                Some(d) => d,
                None => continue,
            },
            None => match expand_path(path, &paths.home) {
                Some(d) => d,
                None => continue,
            },
        };
        let item_dir = store.join(store::item_rel_path(id));
        let (dest, src, mode) = match layout {
            Layout::DirPerItem => (dir.join(key.name()), item_dir, mode),
            Layout::FilePerItem => {
                let file = item_file_name(key);
                // Junctions only link directories and file symlinks need
                // Developer Mode on Windows, so files are copied there.
                let mode = if cfg!(windows) { LinkMode::Copy } else { mode };
                (dir.join(&file), item_dir.join(&file), mode)
            }
        };
        out.push(Desired {
            item: key.clone(),
            dest,
            src,
            mode,
            content_hash: items[id].content_hash.clone(),
        });
    }
    out.sort_by(|a, b| a.dest.cmp(&b.dest));
    out
}

/// Counts a plan's actions. A kept link whose item content changed (the
/// store entry behind it was rebuilt) counts as updated.
fn summarize(
    id: &str,
    plan: &links::Plan,
    applied: &link::Applied,
    home: &Path,
    before: &[loadout_core::links::Owned],
) -> TargetReport {
    let failed: BTreeMap<&Path, String> = applied
        .errors
        .iter()
        .map(|(p, e)| (p.as_path(), e.to_string()))
        .collect();
    let mut t = TargetReport {
        id: id.to_owned(),
        ..TargetReport::default()
    };
    for action in &plan.actions {
        let dest = action.dest();
        if let Some(msg) = failed.get(dest) {
            t.errors.push(PathReport {
                item: action_item(action),
                path: display_path(dest, home),
                message: msg.clone(),
            });
            continue;
        }
        match action {
            Action::Create(d) => t.added.push(d.item.to_string()),
            Action::Replace(d) => t.updated.push(d.item.to_string()),
            Action::Keep(d) => {
                let changed = before
                    .iter()
                    .any(|o| o.dest == d.dest && o.content_hash != d.content_hash);
                if changed {
                    t.updated.push(d.item.to_string());
                } else {
                    t.unchanged += 1;
                }
            }
            Action::Remove(o) => t.removed.push(o.item.to_string()),
            Action::Collision { wanted, .. } => t.collisions.push(PathReport {
                item: wanted.item.to_string(),
                path: display_path(dest, home),
                message: "exists and is not managed by Loadout".into(),
            }),
            Action::Disown { .. } => {}
        }
    }
    t
}

fn action_item(a: &Action) -> String {
    match a {
        Action::Create(d) | Action::Replace(d) | Action::Keep(d) => d.item.to_string(),
        Action::Collision { wanted, .. } => wanted.item.to_string(),
        Action::Remove(o) | Action::Disown { owned: o, .. } => o.item.to_string(),
    }
}

/// Shows paths under the home directory as `~/…` with `/` separators.
pub fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => {
            let parts: Vec<String> = rest
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            if parts.is_empty() {
                "~".into()
            } else {
                format!("~/{}", parts.join("/"))
            }
        }
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_path_uses_tilde() {
        let home = Path::new("/home/u");
        assert_eq!(
            display_path(&home.join(".claude").join("skills"), home),
            "~/.claude/skills"
        );
        assert_eq!(display_path(home, home), "~");
        assert_eq!(display_path(Path::new("/etc/x"), home), "/etc/x");
    }
}

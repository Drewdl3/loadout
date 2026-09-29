//! Loading sources for an apply: which repos, at which commit, and — for
//! fetched changes — whether they apply now or wait for review.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use loadout_audit::Finding;
use loadout_core::resolve::Placement;
use loadout_git::Git;
use loadout_model::config::normalize_url;
use loadout_model::{Config, Lock, RankOrigin, SourceSub};

use crate::membership::{self, LoadedCompanyConfig};
use crate::paths::Paths;
use crate::review::{self, Change, FindingRecord, PendingChange, Reviewer, Verdict};
use crate::scan::{Overrides, ScannedSource, scan_source_with};
use crate::signing::Signing;

/// Maximum concurrent fetches.
const MAX_FETCHES: usize = 8;
/// How many `upstream:` hops are followed from a subscribed source.
pub const MAX_UPSTREAM_DEPTH: usize = 8;

/// Whether to update source checkouts before applying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetch {
    /// Fetch every source and review changes (`lo sync`).
    Remote,
    /// Use the applied commits from `loadout.lock` (toggles, preferences,
    /// approvals).
    Cached,
    /// Check out exactly the commits in `loadout.lock` (`sync --locked`);
    /// fetch a pinned commit only if it is missing locally.
    Locked,
}

/// How to load: the mode plus review inputs.
pub struct LoadOptions<'a> {
    pub fetch: Fetch,
    pub lock: &'a Lock,
    /// Decides on fetched changes (`Fetch::Remote`).
    pub reviewer: Option<&'a Reviewer<'a>>,
    /// Normalized URL → commit to install instead of the lock's
    /// (`lo approve`).
    pub approved: &'a BTreeMap<String, String>,
    /// Plan only: leave every checkout at its applied commit.
    pub dry_run: bool,
    /// Layers that need signed commits, and the allowed keys.
    pub signing: Option<&'a Signing>,
}

/// A source to load, where it came from, and how.
pub struct Planned {
    pub sub: SourceSub,
    /// A manual subscription (not company config-derived).
    pub manual: bool,
    /// The source whose `upstream:` pulled this one in.
    pub via: Option<String>,
    /// Where its subscriber put it (`layer` / `rank`), overriding its items'
    /// own layers.
    pub placement: Option<Placement>,
    /// Display name from its subscription.
    pub label: Option<String>,
}

impl Planned {
    fn new(sub: SourceSub, manual: bool) -> Self {
        Planned {
            sub,
            manual,
            via: None,
            placement: None,
            label: None,
        }
    }
}

/// A loaded source at the commit being installed.
pub struct Fetched {
    pub sub: SourceSub,
    pub commit: String,
    /// True when nothing was fetched (cached mode, or the fetch failed).
    pub cached: bool,
    /// A manual subscription (not company config-derived).
    pub manual: bool,
    pub scanned: ScannedSource,
    /// The previously applied commit, when this load moves to a new one.
    pub updated_from: Option<String>,
    /// The source whose `upstream:` pulled this one in.
    pub via: Option<String>,
    pub placement: Option<Placement>,
    pub label: Option<String>,
}

/// Everything loading produced.
#[derive(Default)]
pub struct Loaded {
    pub sources: Vec<Fetched>,
    /// Changes held for review.
    pub pending: Vec<PendingChange>,
    /// Audit findings on changes that were applied.
    pub findings: Vec<Finding>,
    /// Updates refused because their commit isn't signed by an allowed key.
    pub signature_errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Company config (company layer) + the sources of every group in the profile +
/// manual subscriptions (subject to `allow_manual_sources`). A `[[source]]`
/// for a company config-listed URL only contributes where it sits (label,
/// layer, rank, priority) and its ref.
pub fn source_list(
    config: &Config,
    company_config: Option<&LoadedCompanyConfig>,
    warnings: &mut Vec<String>,
) -> Vec<Planned> {
    let mut out: Vec<Planned> = Vec::new();
    if let Some(c) = company_config {
        let ch = &c.company_config;
        out.push(Planned::new(company_config_sub(c), false));
        let profile = membership::cached_profile(config);
        for group in ch
            .groups
            .iter()
            .filter(|u| profile.contains(&u.layer, &u.name))
        {
            for url in &group.sources {
                out.push(Planned::new(
                    SourceSub {
                        layer: Some(group.layer.clone()),
                        group: Some(group.name.clone()),
                        ..SourceSub::new(url.clone())
                    },
                    false,
                ));
            }
        }
    }
    for sub in &config.sources {
        let url = normalize_url(&sub.url);
        let placement = placement_of(sub.layer.as_deref(), sub.rank, None);
        if let Some(existing) = out.iter_mut().find(|p| normalize_url(&p.sub.url) == url) {
            existing.sub.priority = sub.priority;
            if sub.git_ref.is_some() {
                existing.sub.git_ref.clone_from(&sub.git_ref);
            }
            existing.label.clone_from(&sub.label);
            existing.placement = allowed_placement(placement, &sub.url, company_config, warnings);
            continue;
        }
        if let Some(c) = company_config
            && !c.company_config.policy.allow_manual_sources
            && !c.company_config.lists_source(&sub.url)
        {
            warnings.push(format!(
                "{}: the company config does not allow manual sources; ignored",
                sub.url
            ));
            continue;
        }
        out.push(Planned {
            placement,
            label: sub.label.clone(),
            ..Planned::new(sub.clone(), true)
        });
    }
    // The same URL listed by two groups: keep the first.
    let mut seen = BTreeSet::new();
    out.retain(|p| seen.insert(normalize_url(&p.sub.url)));
    out
}

/// A subscription's `layer` / `rank` as a placement, if it sets either.
fn placement_of(layer: Option<&str>, rank: Option<i64>, via: Option<&str>) -> Option<Placement> {
    (layer.is_some() || rank.is_some()).then(|| Placement {
        layer: layer.map(str::to_owned),
        rank,
        from: RankOrigin::Subscription {
            upstream_of: via.map(str::to_owned),
        },
    })
}

/// `placement`, unless the company config lists `url` (or is it) and
/// forbids re-ranking its sources.
fn allowed_placement(
    placement: Option<Placement>,
    url: &str,
    company_config: Option<&LoadedCompanyConfig>,
    warnings: &mut Vec<String>,
) -> Option<Placement> {
    let Some(c) = company_config else {
        return placement;
    };
    if placement.is_some() && c.lists_or_is(url) && !c.company_config.policy.allow_local_ranks {
        warnings.push(format!(
            "{url}: the company config doesn't allow re-ranking the sources it lists; its layer and rank are kept"
        ));
        return None;
    }
    placement
}

/// The company config repo as a source: company layer, the company group.
pub fn company_config_sub(c: &LoadedCompanyConfig) -> SourceSub {
    SourceSub {
        layer: Some("company".into()),
        group: Some(c.company_config.name.clone()),
        ..SourceSub::new(c.url.clone())
    }
}

/// Loads every planned source. Any source that can't be loaded at all
/// aborts the apply (nothing installed changes).
pub fn load_sources(
    git: &Git,
    paths: &Paths,
    sources: &[Planned],
    opts: &LoadOptions<'_>,
) -> Result<Loaded> {
    let (mut loaded, errors) = load_all(git, paths, sources, opts);
    if !errors.is_empty() {
        let hint = match opts.fetch {
            Fetch::Remote | Fetch::Locked => "",
            Fetch::Cached => " (run `lo sync` first)",
        };
        bail!(
            "could not load {} source(s){hint}; no installed items were changed:\n  {}",
            errors.len(),
            errors.join("\n  ")
        );
    }
    sort_sources(&mut loaded.sources);
    Ok(loaded)
}

/// Loads `sources` in parallel; returns what loaded and one error line per
/// source that didn't.
fn load_all(
    git: &Git,
    paths: &Paths,
    sources: &[Planned],
    opts: &LoadOptions<'_>,
) -> (Loaded, Vec<String>) {
    let results = parallel_map(sources, MAX_FETCHES, |p| {
        load_one(git, paths, &p.sub, p.manual, opts)
    });
    let mut loaded = Loaded::default();
    let mut errors = Vec::new();
    for (p, result) in sources.iter().zip(results) {
        match result {
            Ok(one) => {
                loaded.warnings.extend(one.warning);
                if let Some(mut f) = one.fetched {
                    loaded.warnings.extend(
                        f.scanned
                            .warnings
                            .iter()
                            .map(|w| format!("{}: {w}", f.scanned.manifest.name)),
                    );
                    f.via.clone_from(&p.via);
                    f.placement.clone_from(&p.placement);
                    f.label.clone_from(&p.label);
                    loaded.sources.push(f);
                }
                loaded.pending.extend(one.pending);
                loaded.findings.extend(one.findings);
                loaded.signature_errors.extend(one.signature_error);
            }
            Err(e) => errors.push(format!("{}: {e:#}", p.sub.url)),
        }
    }
    (loaded, errors)
}

pub fn sort_sources(sources: &mut [Fetched]) {
    sources.sort_by(|a, b| {
        (a.scanned.manifest.name.as_str(), a.sub.url.as_str())
            .cmp(&(b.scanned.manifest.name.as_str(), b.sub.url.as_str()))
    });
}

/// The sources named in the `upstream:` of `fetched` that aren't in `seen`
/// yet. An upstream the company config lists gets that group's layer and group and
/// counts as company config-derived; any other one is a manual source (subject to
/// `allow_manual_sources`, never auto-applied). The entry's `label`,
/// `layer`, `rank` and `priority` say where it sits for this source's
/// subscribers; a `[[source]]` of your own for the URL wins over them (it
/// is already in `seen`).
pub fn upstream_plan(
    fetched: &[&Fetched],
    seen: &mut BTreeSet<String>,
    company_config: Option<&LoadedCompanyConfig>,
    warnings: &mut Vec<String>,
) -> Vec<Planned> {
    let mut out = Vec::new();
    for f in fetched {
        for up in &f.scanned.manifest.upstream {
            let url = up.resolve(&f.sub.url);
            if !seen.insert(normalize_url(&url)) {
                continue;
            }
            let from = &f.scanned.manifest.name;
            let listed = company_config.and_then(|c| {
                c.company_config.groups.iter().find(|u| {
                    u.sources
                        .iter()
                        .any(|s| normalize_url(s) == normalize_url(&url))
                })
            });
            if listed.is_none()
                && let Some(c) = company_config
                && !c.company_config.policy.allow_manual_sources
            {
                warnings.push(format!(
                    "{from}: upstream {url} is not in the company config, which does not allow other sources; ignored"
                ));
                continue;
            }
            let placement = allowed_placement(
                placement_of(up.layer(), up.rank(), Some(from)),
                &url,
                company_config,
                warnings,
            );
            let sub = SourceSub {
                layer: listed.map(|u| u.layer.clone()),
                group: listed.map(|u| u.name.clone()),
                git_ref: up.git_ref().map(str::to_owned),
                priority: up.priority().unwrap_or(0),
                ..SourceSub::new(url)
            };
            out.push(Planned {
                via: Some(from.clone()),
                placement,
                label: up.label().map(str::to_owned),
                ..Planned::new(sub, listed.is_none())
            });
        }
    }
    out
}

/// Follows `upstream:` from every loaded source, level by level, up to
/// [`MAX_UPSTREAM_DEPTH`]. `seen` holds the (normalized) URLs already
/// planned. An upstream that can't be loaded is skipped with a warning:
/// someone else's manifest shouldn't break your whole sync.
pub fn load_upstreams(
    git: &Git,
    paths: &Paths,
    loaded: &mut Loaded,
    seen: &mut BTreeSet<String>,
    company_config: Option<&LoadedCompanyConfig>,
    opts: &LoadOptions<'_>,
) {
    let mut frontier: Vec<usize> = (0..loaded.sources.len()).collect();
    for depth in 0..=MAX_UPSTREAM_DEPTH {
        let parents: Vec<&Fetched> = frontier.iter().map(|&i| &loaded.sources[i]).collect();
        let mut warnings = Vec::new();
        let planned = upstream_plan(&parents, seen, company_config, &mut warnings);
        loaded.warnings.append(&mut warnings);
        if planned.is_empty() {
            break;
        }
        if depth == MAX_UPSTREAM_DEPTH {
            loaded.warnings.push(format!(
                "upstream chain deeper than {MAX_UPSTREAM_DEPTH} levels; not following {}",
                planned
                    .iter()
                    .map(|p| p.sub.url.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            break;
        }
        let (more, errors) = load_all(git, paths, &planned, opts);
        loaded
            .warnings
            .extend(errors.into_iter().map(|e| format!("upstream {e}; skipped")));
        let start = loaded.sources.len();
        loaded.sources.extend(more.sources);
        loaded.pending.extend(more.pending);
        loaded.findings.extend(more.findings);
        loaded.signature_errors.extend(more.signature_errors);
        loaded.warnings.extend(more.warnings);
        frontier = (start..loaded.sources.len()).collect();
    }
    sort_sources(&mut loaded.sources);
    fill_from_upstream_entries(loaded, company_config);
}

/// A source you subscribed to yourself (not one the company config lists)
/// that another loaded source also
/// names as `upstream:` takes that entry's label, layer, rank and priority
/// for whatever your own `[[source]]` doesn't set: labelling an upstream
/// shouldn't drop the rank its downstream gave it.
fn fill_from_upstream_entries(loaded: &mut Loaded, company_config: Option<&LoadedCompanyConfig>) {
    // URL → (the entry, the source naming it); the first by source name,
    // so the result doesn't depend on load order.
    let mut entries: BTreeMap<String, (loadout_model::Upstream, String)> = BTreeMap::new();
    for f in &loaded.sources {
        for up in &f.scanned.manifest.upstream {
            entries
                .entry(normalize_url(&up.resolve(&f.sub.url)))
                .or_insert_with(|| (up.clone(), f.scanned.manifest.name.clone()));
        }
    }
    let mut warnings = Vec::new();
    for f in loaded
        .sources
        .iter_mut()
        .filter(|f| f.manual && f.via.is_none())
    {
        let Some((up, from)) = entries.get(&normalize_url(&f.sub.url)) else {
            continue;
        };
        if f.label.is_none() {
            f.label = up.label().map(str::to_owned);
        }
        if f.placement.is_none() {
            f.placement = allowed_placement(
                placement_of(up.layer(), up.rank(), Some(from)),
                &f.sub.url,
                company_config,
                &mut warnings,
            );
        }
        if f.sub.priority == 0 {
            f.sub.priority = up.priority().unwrap_or(0);
        }
    }
    loaded.warnings.append(&mut warnings);
}

/// The result of loading one source.
#[derive(Default)]
pub struct One {
    /// `None` when a new source is held for review (nothing to install).
    pub fetched: Option<Fetched>,
    pub pending: Option<PendingChange>,
    pub findings: Vec<Finding>,
    pub warning: Option<String>,
    /// The fetched commit failed signature verification and was refused.
    pub signature_error: Option<String>,
}

/// The applied commit of `url`: its lock entry. Before the first locked
/// apply (an empty lock), the current checkout counts as applied.
pub fn applied_commit(git: &Git, paths: &Paths, lock: &Lock, url: &str) -> Option<String> {
    match lock.source(url) {
        Some(s) => Some(s.commit.clone()),
        None if lock.sources.is_empty() => git.head(&repo_dir(paths, url)).ok(),
        None => None,
    }
}

pub fn repo_dir(paths: &Paths, url: &str) -> std::path::PathBuf {
    loadout_git::repo_dir(&paths.repos_dir(), &normalize_url(url))
}

pub fn load_one(
    git: &Git,
    paths: &Paths,
    sub: &SourceSub,
    manual: bool,
    opts: &LoadOptions<'_>,
) -> Result<One> {
    let dir = repo_dir(paths, &sub.url);
    let overrides = Overrides {
        layer: sub.layer.clone(),
        group: sub.group.clone(),
    };
    let fetched =
        |commit: String, cached: bool, scanned: ScannedSource, from: Option<String>| Fetched {
            sub: sub.clone(),
            commit,
            cached,
            manual,
            scanned,
            updated_from: from,
            via: None,
            placement: None,
            label: None,
        };
    let applied = applied_commit(git, paths, opts.lock, &sub.url);
    match opts.fetch {
        Fetch::Locked => {
            let pin = opts
                .lock
                .source(&sub.url)
                .context("not in loadout.lock (run `lo sync` without --locked to add it)")?;
            let commit = pin_checkout(git, paths, &sub.url, &pin.commit)?;
            let scanned = scan_source_with(&dir, &overrides)?;
            Ok(One {
                fetched: Some(fetched(commit, true, scanned, None)),
                ..One::default()
            })
        }
        Fetch::Cached => {
            let approved = opts.approved.get(&normalize_url(&sub.url));
            let Some(commit) = approved.cloned().or(applied.clone()) else {
                return Ok(One {
                    warning: Some(format!(
                        "{}: not installed yet; run `lo sync` (or `lo approve`)",
                        sub.url
                    )),
                    ..One::default()
                });
            };
            pin_checkout(git, paths, &sub.url, &commit)?;
            let scanned = scan_source_with(&dir, &overrides)?;
            let from = approved.and(applied).filter(|a| a != &commit);
            Ok(One {
                fetched: Some(fetched(commit, true, scanned, from)),
                ..One::default()
            })
        }
        Fetch::Remote => {
            let reviewer = opts.reviewer.context("remote loads need a reviewer")?;
            // The applied state, before looking upstream.
            let old = match &applied {
                Some(c) => {
                    pin_checkout(git, paths, &sub.url, c)?;
                    Some(scan_source_with(&dir, &overrides)?)
                }
                None => None,
            };
            let to = match git.fetch(&sub.url, &dir, sub.git_ref.as_deref()) {
                Ok(c) => c,
                Err(e) => {
                    let (Some(c), Some(o)) = (applied, old) else {
                        return Err(e.into());
                    };
                    return Ok(One {
                        fetched: Some(fetched(c, true, o, None)),
                        warning: Some(format!(
                            "{}: fetch failed, using cached checkout ({e})",
                            sub.url
                        )),
                        ..One::default()
                    });
                }
            };
            if applied.as_deref() == Some(to.as_str())
                && let Some(o) = old
            {
                return Ok(One {
                    fetched: Some(fetched(to, false, o, None)),
                    ..One::default()
                });
            }
            git.checkout(&dir, &to)?;
            let new = match scan_source_with(&dir, &overrides) {
                Ok(s) => s,
                Err(e) => {
                    let (Some(c), Some(o)) = (applied, old) else {
                        return Err(e);
                    };
                    git.checkout(&dir, &c)?;
                    return Ok(One {
                        fetched: Some(fetched(c.clone(), true, o, None)),
                        warning: Some(format!(
                            "{}: new commit {} is invalid ({e:#}); keeping {}",
                            sub.url,
                            short(&to),
                            short(&c)
                        )),
                        ..One::default()
                    });
                }
            };
            // Nested sources share the repo's commits, so one layer that
            // needs signing covers the whole repo.
            if let Some(sig) = opts.signing
                && let Some(layer) = new.layers().find(|l| sig.requires(l))
                && let Err(e) = git.verify_commit(&dir, &to, &sig.keys)
            {
                let kept = match &applied {
                    Some(c) => format!("keeping {}", short(c)),
                    None => "nothing installed from it".to_owned(),
                };
                let error = Some(format!(
                    "{}: REFUSED commit {}: not signed by an allowed signer ({} layer requires signed commits); {kept}. {}",
                    sub.url,
                    short(&to),
                    layer,
                    e.to_string().lines().last().unwrap_or_default()
                ));
                return Ok(match (applied, old) {
                    (Some(c), Some(o)) => {
                        git.checkout(&dir, &c)?;
                        One {
                            fetched: Some(fetched(c, false, o, None)),
                            signature_error: error,
                            ..One::default()
                        }
                    }
                    _ => One {
                        signature_error: error,
                        ..One::default()
                    },
                });
            }
            let (verdict, findings) = reviewer.decide(&Change {
                url: &sub.url,
                manual,
                old: old.as_ref(),
                new: &new,
            });
            match verdict {
                Verdict::Apply => {
                    if opts.dry_run
                        && let Some(c) = &applied
                    {
                        git.checkout(&dir, c)?;
                    }
                    Ok(One {
                        fetched: Some(fetched(to, false, new, applied)),
                        findings,
                        ..One::default()
                    })
                }
                Verdict::Hold(reason) => {
                    let pending = PendingChange {
                        source: new.manifest.name.clone(),
                        url: sub.url.clone(),
                        layer: new.default_layer.clone(),
                        from: applied.clone(),
                        to,
                        reason,
                        items: review::item_changes(old.as_ref(), &new),
                        findings: findings.iter().map(FindingRecord::from).collect(),
                    };
                    let fetched = match (applied, old) {
                        (Some(c), Some(o)) => {
                            git.checkout(&dir, &c)?;
                            Some(fetched(c, false, o, None))
                        }
                        _ => None,
                    };
                    Ok(One {
                        fetched,
                        pending: Some(pending),
                        ..One::default()
                    })
                }
            }
        }
    }
}

pub fn short(commit: &str) -> &str {
    &commit[..commit.len().min(8)]
}

/// Reads `loadout.lock`; a missing file is an empty lock.
pub fn load_lock(paths: &Paths) -> Result<Lock> {
    let path = paths.lock_file();
    match std::fs::read_to_string(&path) {
        Ok(t) => Lock::parse(&t).with_context(|| format!("in {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Lock::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Checks out `commit` of `url`'s clone, fetching exactly that commit if
/// this machine doesn't have it yet.
pub fn pin_checkout(git: &Git, paths: &Paths, url: &str, commit: &str) -> Result<String> {
    let dir = repo_dir(paths, url);
    if !git.has_commit(&dir, commit) {
        git.fetch_commit(url, &dir, commit)
            .with_context(|| format!("fetching pinned commit {commit}"))?;
    }
    if git.head(&dir).ok().as_deref() != Some(commit) {
        git.checkout(&dir, commit)?;
    }
    Ok(commit.to_owned())
}

/// Runs `f` over `items` on up to `workers` threads, preserving order.
pub fn parallel_map<T: Sync, R: Send>(
    items: &[T],
    workers: usize,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<R>>> = Mutex::new(items.iter().map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..workers.min(items.len()) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = f(item);
                    results.lock().unwrap()[i] = Some(r);
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|r| r.expect("every item processed"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_map_preserves_order() {
        let input: Vec<u32> = (0..50).collect();
        let out = parallel_map(&input, 4, |x| x * 2);
        assert_eq!(out, input.iter().map(|x| x * 2).collect::<Vec<_>>());
        assert!(parallel_map(&[] as &[u32], 4, |x| *x).is_empty());
    }
}

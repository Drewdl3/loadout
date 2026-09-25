//! `lo subscribe` / `lo unsubscribe` — manual sources.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Result, bail};
use clap::Args;
use loadout_model::SourceSub;
use schemars::JsonSchema;
use serde::Serialize;

use crate::ctx::{Ctx, Report};
use crate::exit;

#[derive(Debug, Args)]
pub struct SubscribeArgs {
    /// Git URL of the source repo, or the path to a local Git repo.
    pub url: String,
    /// Layer to assign to the source's items when `LOADOUT.md` doesn't.
    #[arg(long)]
    pub layer: Option<String>,
    /// Group to assign to the source's items when `LOADOUT.md` doesn't.
    #[arg(long)]
    pub group: Option<String>,
    /// Tie-breaker among equal-rank sources; higher wins.
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub priority: i64,
    /// Branch, tag or commit to track instead of the default branch.
    #[arg(long = "ref")]
    pub git_ref: Option<String>,
    /// Only show what the source (and its upstreams) would bring; change
    /// nothing.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct UnsubscribeArgs {
    /// URL of the source, as subscribed.
    pub url: String,
}

/// `--json` output of `lo subscribe` and `lo unsubscribe`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SubscriptionReport {
    pub url: String,
    /// Whether the config changed (false: already subscribed).
    pub changed: bool,
    #[serde(skip)]
    #[schemars(skip)]
    subscribe: bool,
}

impl Report for SubscriptionReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        match (self.subscribe, self.changed) {
            (true, true) => {
                writeln!(out, "Subscribed to {}.", self.url)?;
                writeln!(out, "Run `lo sync` to install its items.")
            }
            (true, false) => writeln!(out, "Already subscribed to {}.", self.url),
            (false, _) => {
                writeln!(out, "Unsubscribed from {}.", self.url)?;
                writeln!(out, "Run `lo sync` to remove its items.")
            }
        }
    }
}

/// A source or company config URL as given by the user: local paths must be the
/// top of a Git repository, and are returned absolute (see
/// [`canonical_url`]). Catches a mistyped or non-Git folder here instead
/// of in `git fetch`.
pub fn checked_url(url: &str) -> Result<String> {
    let url = url.trim();
    if !looks_local(url) {
        return Ok(url.to_owned());
    }
    let path = expand_home(url);
    let abs = tidy(&std::path::absolute(&path).unwrap_or_else(|_| path.clone()));
    if !path.exists() {
        let cwd = std::env::current_dir()
            .ok()
            .filter(|_| path.is_relative())
            .map(|d| format!(" (relative to {})", d.display()))
            .unwrap_or_default();
        bail!(
            "no folder at {}{cwd}; give a Git URL or the path to a local Git repository",
            abs.display()
        );
    }
    if !is_git_root(&path) {
        let inside = abs
            .ancestors()
            .skip(1)
            .find(|a| a.join(".git").exists())
            .map(|root| format!(" It is inside the repository at {}, but a source must be a repository's top folder.", root.display()))
            .unwrap_or_default();
        bail!(
            "{} is not a Git repository.{inside} Sources are Git repos: run `git init && git add -A && git commit -m \"Add source\"` in it, or create one with `lo init --new-source <dir> --git-init`",
            abs.display()
        );
    }
    if loadout_git::Git::new()
        .run(Some(&path), ["rev-parse", "--verify", "--quiet", "HEAD"])
        .is_err()
    {
        bail!(
            "{} has no commits yet; commit its files first (`git add -A && git commit -m \"Add source\"`)",
            abs.display()
        );
    }
    Ok(canonical_url(&abs.to_string_lossy()))
}

/// A path rather than a URL: `./x`, `../x`, `/x`, `~/x`, `C:\x`, or anything
/// that exists on disk. `host:path` (scp-style Git) is a URL.
fn looks_local(url: &str) -> bool {
    !url.contains("://")
        && (url.starts_with('.')
            || url.starts_with('/')
            || url.starts_with('~')
            || Path::new(url).is_absolute()
            || Path::new(url).exists())
}

/// `a/b/../c` → `a/c`, without touching the filesystem.
fn tidy(p: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

fn expand_home(url: &str) -> std::path::PathBuf {
    let rest = url
        .strip_prefix("~/")
        .or_else(|| url.strip_prefix("~\\"))
        .or_else(|| (url == "~").then_some(""));
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match (rest, home) {
        (Some(rest), Some(home)) => Path::new(&home).join(rest),
        _ => Path::new(url).to_owned(),
    }
}

/// The top of a work tree (`.git` dir, or file for worktrees/submodules),
/// or a bare repository.
fn is_git_root(dir: &Path) -> bool {
    dir.join(".git").exists() || (dir.join("HEAD").is_file() && dir.join("objects").is_dir())
}

/// Local paths are stored absolute so fetches work from any directory.
pub fn canonical_url(url: &str) -> String {
    let p = Path::new(url);
    if !url.contains("://")
        && p.exists()
        && let Ok(abs) = std::path::absolute(p)
    {
        return abs.to_string_lossy().into_owned();
    }
    url.to_owned()
}

pub fn subscribe(ctx: &Ctx, args: SubscribeArgs) -> Result<u8> {
    if args.dry_run {
        let report = preview(ctx, &checked_url(&args.url)?, args.git_ref.as_deref())?;
        ctx.emit(&report)?;
        return Ok(exit::OK);
    }
    let (url, changed) = add_subscription(ctx, args)?;
    ctx.emit(&SubscriptionReport {
        url,
        changed,
        subscribe: true,
    })?;
    Ok(exit::OK)
}

/// Adds the subscription to `config.toml`; returns the stored URL and
/// whether the config changed.
pub fn add_subscription(ctx: &Ctx, args: SubscribeArgs) -> Result<(String, bool)> {
    if args.url.trim().is_empty() {
        bail!("source URL must not be empty");
    }
    let url = checked_url(&args.url)?;
    let mut doc = ctx.load_config_doc()?;
    if let Some(company_config_url) = doc.config().company_config {
        // Enforce the company config's policy using the local checkout.
        let loaded = crate::membership::load_company_config(
            ctx,
            &loadout_git::Git::new(),
            &company_config_url,
            false,
        )?;
        if !loaded.company_config.policy.allow_manual_sources
            && !loaded.company_config.lists_source(&url)
        {
            bail!(
                "your company config does not allow subscribing to sources it doesn't list ({url})"
            );
        }
    }
    // In a project layer, sources default to the project's own group.
    let (layer, group) = match (&ctx.paths.project, args.layer, args.group) {
        (Some(p), None, None) => (
            Some("project".to_owned()),
            Some(crate::commands::project::group_name(&p.root)),
        ),
        (_, layer, group) => (layer, group),
    };
    let changed = doc.add_source(&SourceSub {
        url: url.clone(),
        layer,
        group,
        priority: args.priority,
        git_ref: args.git_ref,
    });
    if changed {
        ctx.save_config(&doc)?;
    }
    Ok((url, changed))
}

pub fn unsubscribe(ctx: &Ctx, args: UnsubscribeArgs) -> Result<u8> {
    let url = canonical_url(&args.url);
    let mut doc = ctx.load_config_doc()?;
    if !doc.remove_source(&url) {
        bail!("not subscribed to {url} (see `config.toml` for manual sources)");
    }
    ctx.save_config(&doc)?;
    ctx.emit(&SubscriptionReport {
        url,
        changed: true,
        subscribe: false,
    })?;
    Ok(exit::OK)
}

/// One source in a preview.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewSource {
    pub url: String,
    pub name: String,
    pub layer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub commit: String,
    /// The source whose `upstream:` brings this one in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    pub items: Vec<loadout_search::Doc>,
    /// Template ids (`source:template/name`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<String>,
}

/// `--json` output of `lo subscribe --dry-run`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SubscribePreview {
    /// The source first, then its upstreams (transitively).
    pub sources: Vec<PreviewSource>,
    pub warnings: Vec<String>,
}

impl Report for SubscribePreview {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        for s in &self.sources {
            let via = s
                .via
                .as_deref()
                .map(|v| format!("  (upstream of {v})"))
                .unwrap_or_default();
            writeln!(
                out,
                "{} ({}:{}) {}{via}",
                s.name,
                s.layer,
                s.group.as_deref().unwrap_or("-"),
                &s.commit[..s.commit.len().min(8)]
            )?;
            if let Some(d) = &s.description {
                writeln!(out, "  {d}")?;
            }
            for i in &s.items {
                writeln!(
                    out,
                    "  {:<32} {}",
                    format!("{}/{}", i.kind, i.name),
                    i.description.as_deref().unwrap_or("")
                )?;
            }
            for t in &s.templates {
                writeln!(out, "  {t}")?;
            }
        }
        writeln!(
            out,
            "Nothing was changed. Subscribe with `lo subscribe <url>`."
        )
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// Fetches `url` (and its upstreams) into a preview cache, separate from
/// the checkouts of installed sources, and describes what they contain.
pub fn preview(ctx: &Ctx, url: &str, git_ref: Option<&str>) -> Result<SubscribePreview> {
    let git = loadout_git::Git::new();
    let cache = ctx.paths.data_dir.join("preview");
    let mut out = SubscribePreview {
        sources: Vec::new(),
        warnings: Vec::new(),
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut queue: std::collections::VecDeque<(String, Option<String>, Option<String>, usize)> =
        std::collections::VecDeque::from([(url.to_owned(), git_ref.map(str::to_owned), None, 0)]);
    while let Some((url, git_ref, via, depth)) = queue.pop_front() {
        if !seen.insert(loadout_model::config::normalize_url(&url)) {
            continue;
        }
        let dir = loadout_git::repo_dir(&cache, &loadout_model::config::normalize_url(&url));
        let commit = match git.sync_checkout(&url, &dir, git_ref.as_deref()) {
            Ok(c) => c.to_string(),
            Err(e) if via.is_none() => bail!("can't read {url}: {e}"),
            Err(e) => {
                out.warnings
                    .push(format!("upstream {url}: can't read ({e})"));
                continue;
            }
        };
        let scanned = match crate::scan::scan_source(&dir) {
            Ok(s) => s,
            Err(e) if via.is_none() => return Err(e.context(format!("{url} is not a source"))),
            Err(e) => {
                out.warnings.push(format!("upstream {url}: {e:#}"));
                continue;
            }
        };
        let templates = crate::scan::scan_templates(&dir)
            .map(|(t, _)| t.into_iter().map(|t| t.id).collect())
            .unwrap_or_default();
        out.warnings.extend(
            scanned
                .warnings
                .iter()
                .map(|w| format!("{}: {w}", scanned.manifest.name)),
        );
        if depth < crate::sources::MAX_UPSTREAM_DEPTH {
            for up in &scanned.manifest.upstream {
                queue.push_back((
                    up.resolve(&url),
                    up.git_ref().map(str::to_owned),
                    Some(scanned.manifest.name.clone()),
                    depth + 1,
                ));
            }
        }
        out.sources.push(PreviewSource {
            url: url.clone(),
            name: scanned.manifest.name.clone(),
            layer: scanned.default_layer.clone(),
            group: scanned.default_group.clone(),
            description: scanned.manifest.description.clone(),
            commit,
            via,
            items: scanned
                .items
                .iter()
                .map(|i| {
                    let mut d = crate::search::doc(i, loadout_search::State::NotSubscribed);
                    d.body.clear();
                    d
                })
                .collect(),
            templates,
        });
    }
    Ok(out)
}

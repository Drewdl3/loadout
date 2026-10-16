//! `lo skills`: a skills manager for everything on this computer.
//!
//! - `lo skills [list]` shows every skill in your AI tools' folders (and
//!   the current project's), whether Loadout installed it, who did, and
//!   what's worth fixing: copies that differ, a tool seeing two, a skill
//!   only some tools have, something blocking a Loadout install.
//! - `lo skills manage` puts skills under Loadout: it follows the repo a
//!   skill came from when its installer recorded one, and otherwise copies
//!   it into your local source. Either way the next sync installs it into
//!   every tool and moves the originals aside (see [`crate::handover`]).
//! - `lo skills unmanage` undoes that.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use loadout_core::frontmatter_edit as fe;
use loadout_git::Git;
use loadout_model::config::normalize_url;
use loadout_model::id::validate_item_name;
use loadout_model::{ItemKey, ItemKind, SourceSub};
use schemars::JsonSchema;
use serde::Serialize;

use crate::ctx::{Ctx, Report};
use crate::handover::{AdoptedFile, Adoption, HandedOver, Restored};
use crate::inventory::{self, Copy, Inventory, Issue, Scope, Skill, State};
use crate::scan::SKILL_FILE;
use crate::{exit, local};

#[derive(Debug, Args)]
pub struct SkillsArgs {
    #[command(subcommand)]
    pub command: Option<SkillsCommand>,
}

#[derive(Debug, Subcommand)]
pub enum SkillsCommand {
    /// Every skill on this computer and what's worth fixing (the default).
    List(ListArgs),
    /// Put skills under Loadout: follow the repo they came from, or copy
    /// them into your local source. Your originals move aside on sync.
    Manage(ManageArgs),
    /// Stop managing a skill and put your originals back.
    Unmanage(UnmanageArgs),
}

#[derive(Debug, Default, Args)]
pub struct ListArgs {
    /// Also look in this project (repo) folder (repeatable). The repo
    /// you're in is included unless `--no-project`.
    #[arg(long = "in", value_name = "DIR")]
    pub projects: Vec<PathBuf>,
    /// Don't look in the repo you're in.
    #[arg(long)]
    pub no_project: bool,
    /// Only skills with something worth fixing.
    #[arg(long)]
    pub issues: bool,
}

#[derive(Debug, Args)]
pub struct ManageArgs {
    /// Skills to manage, by name.
    #[arg(value_name = "NAME", required_unless_present = "all")]
    pub names: Vec<String>,
    /// Every skill Loadout doesn't manage yet.
    #[arg(long, conflicts_with = "names")]
    pub all: bool,
    /// Copy into your local source even when the repo a skill came from is
    /// known (you won't get its updates).
    #[arg(long)]
    pub copy: bool,
    /// Which copy to keep when copies differ (its folder).
    #[arg(long, value_name = "PATH")]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct UnmanageArgs {
    /// The skill to stop managing.
    #[arg(value_name = "NAME")]
    pub name: String,
}

/// `lo skills manage|unmanage --json`.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct SkillsChangeReport {
    pub managed: Vec<Managed>,
    pub skipped: Vec<SkippedSkill>,
    /// `unmanage`: the originals put back.
    pub restored: Vec<Restored>,
    /// Originals the sync moved aside.
    pub handed_over: Vec<HandedOver>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum How {
    /// Its repo is followed as a source.
    Followed,
    /// Copied into your local source.
    Copied,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Managed {
    pub name: String,
    pub how: How,
    /// The source it now comes from.
    pub source: String,
    /// The followed repo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The originals that move aside.
    pub originals: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed_from: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SkippedSkill {
    pub name: String,
    pub reason: String,
}

impl Report for Inventory {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if self.skills.is_empty() {
            return writeln!(out, "No skills found in your AI tools' folders.");
        }
        for s in &self.skills {
            let state = match s.state {
                State::Managed => format!("from {}", s.item.as_deref().unwrap_or("Loadout")),
                State::PartlyManaged => {
                    format!(
                        "from {}, plus other copies",
                        s.item.as_deref().unwrap_or("Loadout")
                    )
                }
                State::Adopted => "adopted, waiting for its source".to_owned(),
                State::Unmanaged => match s.copies.iter().find_map(|c| c.origin.as_ref()) {
                    Some(o) => format!("not managed (from {} via {})", o.source, o.installer),
                    None => "not managed".to_owned(),
                },
            };
            let tools: BTreeSet<&str> = s
                .copies
                .iter()
                .flat_map(|c| c.seen_by.iter().map(|v| v.tool.as_str()))
                .collect();
            writeln!(
                out,
                "{:<28} {:<44} {}",
                s.name,
                state,
                tools.into_iter().collect::<Vec<_>>().join(", ")
            )?;
            for i in &s.issues {
                writeln!(out, "  ! {}", describe(i))?;
            }
        }
        let unmanaged = self
            .skills
            .iter()
            .filter(|s| s.state == State::Unmanaged)
            .count();
        if unmanaged > 0 {
            writeln!(
                out,
                "{unmanaged} not managed: `lo skills manage <name>` (or `--all`) installs them everywhere and keeps one copy."
            )?;
        }
        Ok(())
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// One line about an issue.
pub fn describe(i: &Issue) -> String {
    match i {
        Issue::Blocked { item, tools } => format!(
            "your own copy blocks {item} in {}; manage it to fix",
            tools.join(", ")
        ),
        Issue::Shadowed { tools } => format!("{} see(s) more than one copy", tools.join(", ")),
        Issue::Copies {
            count,
            differ: true,
        } => format!("{count} copies, and they differ"),
        Issue::Copies { count, .. } => format!("{count} identical copies"),
        Issue::Missing { tools } => format!("not in {}", tools.join(", ")),
    }
}

impl Report for SkillsChangeReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        for m in &self.managed {
            let how = match m.how {
                How::Followed => format!("following {}", m.url.as_deref().unwrap_or(&m.source)),
                How::Copied => "copied to your local skills".to_owned(),
            };
            writeln!(out, "Managing {} ({how})", m.name)?;
        }
        for h in &self.handed_over {
            writeln!(out, "  moved your original {} to {}", h.from, h.backup)?;
        }
        for r in &self.restored {
            writeln!(out, "Put {} back at {}", r.name, r.path)?;
        }
        for s in &self.skipped {
            writeln!(out, "Skipped {}: {}", s.name, s.reason)?;
        }
        Ok(())
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

pub fn run(ctx: &Ctx, args: SkillsArgs) -> Result<u8> {
    match args
        .command
        .unwrap_or(SkillsCommand::List(ListArgs::default()))
    {
        SkillsCommand::List(a) => list(ctx, a),
        SkillsCommand::Manage(a) => manage(ctx, a),
        SkillsCommand::Unmanage(a) => unmanage(ctx, a),
    }
}

fn list(ctx: &Ctx, args: ListArgs) -> Result<u8> {
    let mut projects = args.projects.clone();
    if !args.no_project
        && let Some(root) = std::env::current_dir()
            .ok()
            .and_then(|d| inventory::project_root(&d))
        && root != ctx.paths.home
    {
        projects.push(root);
    }
    let projects = projects
        .into_iter()
        .map(|p| p.canonicalize().unwrap_or(p))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut inv = inventory::scan(ctx, &inventory::Options { projects })?;
    if args.issues {
        inv.skills.retain(|s| !s.issues.is_empty());
    }
    ctx.emit(&inv)?;
    Ok(exit::OK)
}

/// A skill to manage, with the copies of it to retire.
type Picked<'a> = (&'a Skill, Vec<&'a Copy>);

/// The global copies to retire when managing `skill`, or why not.
fn originals<'a>(skill: &'a Skill, path: Option<&Path>) -> Result<Vec<&'a Copy>, String> {
    let others: Vec<&Copy> = skill
        .copies
        .iter()
        .filter(|c| !c.managed && c.scope == Scope::Global)
        .collect();
    if others.is_empty() {
        return Err(if skill.copies.iter().all(|c| c.managed) {
            "already managed".to_owned()
        } else {
            "only in a project; share it with `lo adopt <project folder> --into <source>`"
                .to_owned()
        });
    }
    let keep = match path {
        Some(p) => {
            let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            others
                .iter()
                .find(|c| Path::new(&c.path) == p)
                .ok_or_else(|| format!("no copy at {}", p.display()))?
        }
        None => {
            let hashes: BTreeSet<&str> = others.iter().map(|c| c.content_hash.as_str()).collect();
            if hashes.len() > 1 {
                return Err(format!(
                    "its copies differ; pick the one to keep with --path ({})",
                    others
                        .iter()
                        .map(|c| c.path.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            others[0]
        }
    };
    Ok(others
        .iter()
        .copied()
        .filter(|c| c.content_hash == keep.content_hash)
        .collect())
}

fn manage(ctx: &Ctx, args: ManageArgs) -> Result<u8> {
    if ctx.paths.project.is_some() {
        bail!("run `lo skills manage` outside a project; it manages your own skills");
    }
    if args.path.is_some() && args.names.len() != 1 {
        bail!("--path picks the copy of one skill; name exactly one");
    }
    let inv = inventory::scan(ctx, &inventory::Options::default())?;
    let mut report = SkillsChangeReport::default();
    let picked: Vec<&Skill> = if args.all {
        inv.skills
            .iter()
            .filter(|s| matches!(s.state, State::Unmanaged | State::PartlyManaged))
            .filter(|s| {
                s.copies
                    .iter()
                    .any(|c| !c.managed && c.scope == Scope::Global)
            })
            .collect()
    } else {
        let mut out = Vec::new();
        for n in &args.names {
            match inv.skills.iter().find(|s| &s.name == n) {
                Some(s) => out.push(s),
                None => report.skipped.push(SkippedSkill {
                    name: n.clone(),
                    reason: "not found in your AI tools' folders (`lo skills` lists them)".into(),
                }),
            }
        }
        out
    };

    // Follow a skill's repo when its installer recorded one, else copy.
    let mut follow: BTreeMap<String, (String, Vec<Picked<'_>>)> = BTreeMap::new();
    let mut copy: Vec<Picked<'_>> = Vec::new();
    for s in picked {
        let originals = match originals(s, args.path.as_deref()) {
            Ok(o) => o,
            Err(reason) => {
                report.skipped.push(SkippedSkill {
                    name: s.name.clone(),
                    reason,
                });
                continue;
            }
        };
        let origin = originals.iter().find_map(|c| c.origin.as_ref());
        match origin.and_then(|o| Some((o.url.clone()?, o.source.clone()))) {
            Some((url, label)) if !args.copy => follow
                .entry(normalize_url(&url))
                .or_insert_with(|| (label, Vec::new()))
                .1
                .push((s, originals)),
            _ => copy.push((s, originals)),
        }
    }

    let git = Git::new();
    let mut handovers = AdoptedFile::load(&ctx.paths.adopted_file())?;
    let mut doc = ctx.load_config_doc()?;
    for (url, (label, skills)) in follow {
        match follow_repo(ctx, &git, &mut doc, &url, &label, &skills) {
            Ok((source, missing)) => {
                for (s, originals) in &skills {
                    if missing.contains(&s.name) {
                        copy.push((s, originals.clone()));
                        report.warnings.push(format!(
                            "{url} has no skill named {}; copying yours instead",
                            s.name
                        ));
                        continue;
                    }
                    record(&mut handovers, &source, &s.name, originals);
                    report.managed.push(Managed {
                        name: s.name.clone(),
                        how: How::Followed,
                        source: source.clone(),
                        url: Some(url.clone()),
                        originals: originals.iter().map(|c| c.path.clone()).collect(),
                        renamed_from: None,
                    });
                }
            }
            Err(e) => {
                report
                    .warnings
                    .push(format!("can't follow {url} ({e:#}); copying instead"));
                copy.extend(skills);
            }
        }
    }
    ctx.save_config(&doc)?;

    if !copy.is_empty() {
        let dir = local::ensure(ctx, &git)?;
        let mut names = Vec::new();
        for (s, originals) in copy {
            let name = if validate_item_name(&s.name).is_ok() {
                s.name.clone()
            } else {
                crate::scan::kebab(&s.name)
            };
            match copy_skill(&dir, &name, Path::new(&originals[0].path)) {
                Ok(()) => {
                    record(&mut handovers, local::NAME, &name, &originals);
                    names.push(name.clone());
                    report.managed.push(Managed {
                        renamed_from: (name != s.name).then(|| s.name.clone()),
                        name,
                        how: How::Copied,
                        source: local::NAME.to_owned(),
                        url: None,
                        originals: originals.iter().map(|c| c.path.clone()).collect(),
                    });
                }
                Err(e) => report.skipped.push(SkippedSkill {
                    name: s.name.clone(),
                    reason: format!("{e:#}"),
                }),
            }
        }
        if !names.is_empty() {
            local::commit(&git, &dir, &format!("Manage {}", names.join(", ")))?;
        }
    }
    handovers.save(&ctx.paths.adopted_file())?;

    if !report.managed.is_empty() {
        let applied = local::apply(ctx)?;
        report.handed_over = applied.handed_over;
        report.warnings.extend(applied.warnings);
    }
    let code = if report.managed.is_empty() && !report.skipped.is_empty() {
        exit::ERROR
    } else {
        exit::OK
    };
    ctx.emit(&report)?;
    Ok(code)
}

/// Subscribes to `url` (once). A new subscription turns off the repo's
/// other items, so you get the skills you had, not everything in it.
/// Returns the source's name and the wanted skills it doesn't have.
fn follow_repo(
    ctx: &Ctx,
    git: &Git,
    doc: &mut loadout_model::ConfigDoc,
    url: &str,
    label: &str,
    skills: &[Picked<'_>],
) -> Result<(String, BTreeSet<String>)> {
    let dir = loadout_git::repo_dir(&ctx.paths.data_dir.join("preview"), url);
    git.sync_checkout(url, &dir, None)
        .with_context(|| format!("can't read {url}"))?;
    let scanned = crate::scan::scan_source_with(
        &dir,
        &crate::scan::Overrides {
            url: Some(url.to_owned()),
            ..Default::default()
        },
    )?;
    let wanted: BTreeSet<&str> = skills.iter().map(|(s, _)| s.name.as_str()).collect();
    let has = |n: &str| {
        scanned
            .items
            .iter()
            .any(|i| i.id.key().kind() == ItemKind::Skill && i.id.key().name() == n)
    };
    let missing: BTreeSet<String> = wanted
        .iter()
        .filter(|n| !has(n))
        .map(|n| (*n).to_owned())
        .collect();
    if missing.len() == wanted.len() {
        bail!("it has none of these skills");
    }
    let added = doc.add_source(&SourceSub {
        label: Some(label.to_owned()),
        ..SourceSub::new(url)
    });
    if added {
        for i in &scanned.items {
            let key = i.id.key();
            if !(key.kind() == ItemKind::Skill && wanted.contains(key.name())) {
                doc.set_toggle(&i.id.to_string(), false);
            }
        }
    }
    Ok((scanned.manifest.name, missing))
}

fn record(handovers: &mut AdoptedFile, source: &str, name: &str, originals: &[&Copy]) {
    for c in originals {
        handovers.record(Adoption {
            name: name.to_owned(),
            source: source.to_owned(),
            original: PathBuf::from(&c.path),
            content_hash: c.content_hash.clone(),
            backup: None,
            links: Vec::new(),
        });
    }
}

/// Copies a skill folder into the local source as `name`.
fn copy_skill(local: &Path, name: &str, from: &Path) -> Result<()> {
    let key = ItemKey::new(ItemKind::Skill, name)?;
    let (rel, _, _) = crate::work::item_path(local, &key)?;
    let dest = crate::work::join(local, &rel);
    if dest.exists() {
        bail!("your local skills already have {name}");
    }
    let (files, _) = crate::scan::read_tree(from)?;
    for f in &files {
        let mut contents = f.contents.clone();
        if f.path == SKILL_FILE
            && let Ok(text) = String::from_utf8(contents.clone())
            && from.file_name().is_some_and(|n| n != name)
        {
            contents = fe::set_top_key(&text, "name", Some(name)).into_bytes();
        }
        let path = crate::work::join(&dest, &f.path);
        std::fs::create_dir_all(path.parent().expect("has parent"))?;
        std::fs::write(&path, contents)?;
    }
    Ok(())
}

fn unmanage(ctx: &Ctx, args: UnmanageArgs) -> Result<u8> {
    let name = &args.name;
    let adopted = AdoptedFile::load(&ctx.paths.adopted_file())?;
    if !adopted
        .skills
        .iter()
        .any(|a| &a.name == name && a.backup.is_some())
    {
        bail!("Loadout didn't move any original of {name:?} aside; nothing to put back");
    }
    let git = Git::new();
    let mut report = SkillsChangeReport::default();
    // Stop installing it: drop it from the local source, or turn it off.
    let key = ItemKey::new(ItemKind::Skill, name.clone())?;
    let resolved = crate::state::Resolved::load(&ctx.paths.resolved_file())?.unwrap_or_default();
    if let Some(winner) = resolved.resolution.get(&key).and_then(|r| r.winner.clone()) {
        if winner.source() == local::NAME {
            let dir = local::dir(&ctx.paths);
            let (rel, _, _) = crate::work::item_path(&dir, &key)?;
            std::fs::remove_dir_all(crate::work::join(&dir, &rel))?;
            local::commit(&git, &dir, &format!("Stop managing {name}"))?;
        } else {
            let mut doc = ctx.load_config_doc()?;
            doc.set_toggle(&winner.to_string(), false);
            ctx.save_config(&doc)?;
        }
        let applied = local::apply(ctx)?;
        report.warnings.extend(applied.warnings);
    }
    report.restored = crate::handover::restore(&ctx.paths, name, &mut report.warnings)?;
    ctx.emit(&report)?;
    Ok(exit::OK)
}

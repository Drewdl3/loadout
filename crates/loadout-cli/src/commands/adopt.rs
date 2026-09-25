//! `lo adopt` — bring skills that are already installed (in an AI tool's
//! skills directory, or any folder) into a source, so the team can share
//! them through Loadout.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use loadout_git::Git;
use loadout_model::frontmatter::Document;
use loadout_model::id::validate_item_name;
use loadout_model::manifest::MANIFEST_FILE;
use loadout_model::{ItemKey, ItemKind, ItemMeta, ManifestDoc};
use loadout_targets::owned::OwnedFile;
use loadout_targets::{TargetTable, expand_path};
use schemars::JsonSchema;
use serde::Serialize;

use crate::ctx::{Ctx, Report};
use crate::{exit, work};

const SKILL_FILE: &str = "SKILL.md";

#[derive(Debug, Args)]
pub struct AdoptArgs {
    /// Where to look instead of your AI tools' skills directories: a skill
    /// folder (with a SKILL.md) or a folder of skill folders (e.g. a
    /// project's `.claude/skills`).
    #[arg(value_name = "PATH")]
    pub paths: Vec<PathBuf>,
    /// The source (its `LOADOUT.md` name) to add the skills to, in its working
    /// clone; then `lo export-source <source>` opens a pull request.
    #[arg(long, value_name = "SOURCE", conflicts_with = "dir")]
    pub into: Option<String>,
    /// Or: a local checkout of a source repo to copy the skills into.
    #[arg(long, value_name = "PATH")]
    pub dir: Option<PathBuf>,
    /// Adopt this skill (repeatable). In a terminal you're asked instead.
    #[arg(long = "skill", value_name = "NAME")]
    pub skills: Vec<String>,
    /// Adopt every skill found.
    #[arg(long, conflicts_with = "skills")]
    pub all: bool,
}

/// A skill found on disk that Loadout doesn't manage.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Candidate {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The skill's folder.
    pub path: String,
    /// Where it was found: a target id (`claude-code`, …) or `path`.
    pub found_in: String,
    /// Why it can't be adopted as is (e.g. a name that isn't kebab-case).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// One adopted skill.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Adopted {
    pub name: String,
    /// `<source>:skill/<name>`.
    pub id: String,
    /// Where it was copied from.
    pub from: String,
    /// Its folder in the source.
    pub path: String,
    /// Files copied, relative to the skill folder.
    pub files: Vec<String>,
}

/// A skill that was selected but not adopted.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

/// `--json` output of `lo adopt`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AdoptReport {
    /// Skills found. Without `--into`/`--dir`, that's all this command does.
    pub candidates: Vec<Candidate>,
    /// The destination source's name, when adopting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub adopted: Vec<Adopted>,
    pub skipped: Vec<Skipped>,
    /// Written to the source's working clone (`--into`), ready for
    /// `lo export-source`.
    pub working_clone: bool,
    pub warnings: Vec<String>,
}

impl Report for AdoptReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        let Some(source) = &self.source else {
            if self.candidates.is_empty() {
                return writeln!(out, "No skills found that Loadout doesn't already manage.");
            }
            writeln!(out, "Skills you could adopt into a source:")?;
            for c in &self.candidates {
                let note = c.problem.as_deref().map(|p| format!("  [{p}]"));
                writeln!(
                    out,
                    "  {:<28} {:<12} {}{}",
                    c.name,
                    c.found_in,
                    c.path,
                    note.unwrap_or_default()
                )?;
            }
            return writeln!(
                out,
                "Adopt: `lo adopt --into <source> --skill <name>` (or `--all`, or pick in a terminal)."
            );
        };
        if self.adopted.is_empty() {
            writeln!(out, "Nothing adopted into {source}.")?;
        } else {
            writeln!(out, "Adopted into {source}:")?;
            for a in &self.adopted {
                writeln!(out, "  {:<28} from {}", a.id, a.from)?;
            }
        }
        for s in &self.skipped {
            writeln!(out, "  skipped {}: {}", s.name, s.reason)?;
        }
        if !self.adopted.is_empty() {
            if self.working_clone {
                writeln!(
                    out,
                    "Next: `lo export-source {source} -m \"Adopt skills\"` opens a pull request."
                )?;
            } else {
                writeln!(
                    out,
                    "Next: `lo audit --path <checkout>`, then commit and open a pull request."
                )?;
            }
            writeln!(
                out,
                "Once your source ships them, move the originals aside so `lo sync` can install the shared versions."
            )?;
        }
        Ok(())
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

pub fn run(ctx: &Ctx, args: AdoptArgs) -> Result<u8> {
    let mut warnings = Vec::new();
    let candidates = if args.paths.is_empty() {
        installed(ctx, &mut warnings)?
    } else {
        let mut found = Vec::new();
        for p in &args.paths {
            found.extend(in_folder(p, "path", &BTreeSet::new(), &mut warnings)?);
        }
        found
    };
    if args.into.is_none() && args.dir.is_none() {
        if !args.skills.is_empty() || args.all {
            bail!("say where: --into <source> or --dir <path>");
        }
        ctx.emit(&AdoptReport {
            candidates,
            source: None,
            adopted: Vec::new(),
            skipped: Vec::new(),
            working_clone: false,
            warnings,
        })?;
        return Ok(exit::OK);
    }

    let picked = pick(ctx, &args, &candidates)?;
    let git = Git::new();
    let (dest, working_clone) = match (&args.into, &args.dir) {
        (Some(source), _) => (work::open(ctx, &git, source)?.dir, true),
        (None, Some(dir)) => (dir.clone(), false),
        (None, None) => unreachable!("checked above"),
    };
    let manifest = std::fs::read_to_string(dest.join(MANIFEST_FILE)).with_context(|| {
        format!(
            "{} is not a source ({MANIFEST_FILE} missing)",
            dest.display()
        )
    })?;
    let source = ManifestDoc::parse(&manifest)?.manifest.name;

    let mut adopted = Vec::new();
    let mut skipped = Vec::new();
    for c in picked {
        match copy_into(ctx, &dest, &source, &c, &mut warnings) {
            Ok(a) => adopted.push(a),
            Err(e) => skipped.push(Skipped {
                name: c.name.clone(),
                reason: format!("{e:#}"),
            }),
        }
    }
    let code = if adopted.is_empty() && !skipped.is_empty() {
        exit::ERROR
    } else {
        exit::OK
    };
    ctx.emit(&AdoptReport {
        candidates,
        source: Some(source),
        adopted,
        skipped,
        working_clone,
        warnings,
    })?;
    Ok(code)
}

/// The selection: `--all`, `--skill`, or a multi-select in a terminal.
fn pick(ctx: &Ctx, args: &AdoptArgs, candidates: &[Candidate]) -> Result<Vec<Candidate>> {
    if args.all {
        return Ok(candidates.to_vec());
    }
    if !args.skills.is_empty() {
        let mut out = Vec::new();
        for name in &args.skills {
            let c = candidates
                .iter()
                .find(|c| c.name == *name)
                .with_context(|| {
                    format!("no skill named {name:?} found (`lo adopt` lists them)")
                })?;
            out.push(c.clone());
        }
        return Ok(out);
    }
    if ctx.json || !std::io::stdin().is_terminal() {
        bail!("pick skills with --skill <name> (repeatable) or --all");
    }
    if candidates.is_empty() {
        bail!("no skills found that Loadout doesn't already manage");
    }
    let labels: Vec<String> = candidates
        .iter()
        .map(|c| {
            let desc = c.description.as_deref().unwrap_or("");
            let desc: String = desc.chars().take(60).collect();
            match &c.problem {
                Some(p) => format!("{} ({}) — {p}", c.name, c.found_in),
                None => format!("{} ({}) — {desc}", c.name, c.found_in),
            }
        })
        .collect();
    let chosen = dialoguer::MultiSelect::new()
        .with_prompt("Skills to adopt (space to select, enter to confirm)")
        .items(&labels)
        .interact()
        .context("reading your choice")?;
    Ok(chosen.into_iter().map(|i| candidates[i].clone()).collect())
}

/// Skills in every known tool's skills directory that Loadout didn't
/// install. A name found in several tools is listed once (the first).
fn installed(ctx: &Ctx, warnings: &mut Vec<String>) -> Result<Vec<Candidate>> {
    let table = TargetTable::load(&ctx.paths.user_targets_dir())?;
    let owned = OwnedFile::load(&ctx.paths.owned_file()).unwrap_or_default();
    let owned_paths: BTreeSet<PathBuf> = owned
        .targets
        .values()
        .flatten()
        .map(|o| o.dest.clone())
        .collect();
    let mut seen_dirs = BTreeSet::new();
    let mut out: Vec<Candidate> = Vec::new();
    for t in table.iter() {
        let Some(dir) = t
            .skills
            .as_ref()
            .and_then(|s| expand_path(&s.path, &ctx.paths.home))
        else {
            continue;
        };
        if !dir.is_dir() || !seen_dirs.insert(dir.clone()) {
            continue;
        }
        for c in in_folder(&dir, &t.id, &owned_paths, warnings)? {
            if !out.iter().any(|o| o.name == c.name) {
                out.push(c);
            }
        }
    }
    Ok(out)
}

/// `dir` itself when it holds a SKILL.md, else its sub-folders that do.
/// Links (Loadout installs by linking) and `owned` paths are skipped.
fn in_folder(
    dir: &Path,
    found_in: &str,
    owned: &BTreeSet<PathBuf>,
    warnings: &mut Vec<String>,
) -> Result<Vec<Candidate>> {
    if !dir.is_dir() {
        bail!("{} is not a folder", dir.display());
    }
    if dir.join(SKILL_FILE).is_file() {
        return Ok(vec![candidate(dir, found_in, warnings)]);
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    let mut out = Vec::new();
    for e in entries {
        let path = e.path();
        let is_link = e.file_type().map(|t| t.is_symlink()).unwrap_or(false)
            || std::fs::read_link(&path).is_ok();
        if is_link || owned.contains(&path) || !path.join(SKILL_FILE).is_file() {
            continue;
        }
        out.push(candidate(&path, found_in, warnings));
    }
    Ok(out)
}

fn candidate(dir: &Path, found_in: &str, warnings: &mut Vec<String>) -> Candidate {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let meta = std::fs::read_to_string(dir.join(SKILL_FILE))
        .map_err(anyhow::Error::from)
        .and_then(|t| Ok(Document::split(&t)?.parse::<ItemMeta>()?));
    let (description, mut problem) = match meta {
        Ok(m) => (m.description, None),
        Err(e) => {
            warnings.push(format!("{}: {e:#}", dir.join(SKILL_FILE).display()));
            (None, Some("SKILL.md frontmatter can't be read".to_owned()))
        }
    };
    if problem.is_none() && validate_item_name(&name).is_err() {
        problem = Some("name isn't kebab-case; rename the folder first".to_owned());
    }
    Candidate {
        name,
        description,
        path: dir.display().to_string(),
        found_in: found_in.to_owned(),
        problem,
    }
}

fn copy_into(
    ctx: &Ctx,
    dest: &Path,
    source: &str,
    c: &Candidate,
    warnings: &mut Vec<String>,
) -> Result<Adopted> {
    if let Some(p) = &c.problem {
        bail!("{p}");
    }
    let key = ItemKey::new(ItemKind::Skill, c.name.clone())?;
    work::check_editable(ctx, dest, &key)?;
    let (item_rel, _, _) = work::item_path(dest, &key)?;
    let item_dir = work::join(dest, &item_rel);
    if item_dir.exists() {
        bail!("{source} already has {item_rel}");
    }
    let (files, tree_warnings) = crate::scan::read_tree(Path::new(&c.path))?;
    warnings.extend(
        tree_warnings
            .into_iter()
            .map(|w| format!("{}: {w}", c.name)),
    );
    let mut names = Vec::new();
    for f in &files {
        let path = work::join(&item_dir, &f.path);
        std::fs::create_dir_all(path.parent().expect("has parent"))?;
        std::fs::write(&path, &f.contents)
            .with_context(|| format!("writing {}", path.display()))?;
        names.push(f.path.clone());
    }
    names.sort();
    Ok(Adopted {
        name: c.name.clone(),
        id: format!("{source}:skill/{}", c.name),
        from: c.path.clone(),
        path: item_dir.display().to_string(),
        files: names,
    })
}

//! `lo adopt` — bring skills that are already installed (in an AI tool's
//! skills directory, or any folder) into a source, so the team can share
//! them through Loadout.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use loadout_core::frontmatter_edit as fe;
use loadout_git::Git;
use loadout_model::frontmatter::Document;
use loadout_model::id::validate_item_name;
use loadout_model::manifest::MANIFEST_FILE;
use loadout_model::{ItemKey, ItemKind, ItemMeta, ManifestDoc};
use loadout_targets::owned::OwnedFile;
use loadout_targets::{TargetTable, expand_path};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Map, Value};

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
    /// Adopt this skill (repeatable); `NAME=NEW` adopts it under a new
    /// name. In a terminal you're asked instead.
    #[arg(long = "skill", value_name = "NAME[=NEW]")]
    pub skills: Vec<String>,
    /// Adopt every skill found (except ones already adopted or shipped).
    #[arg(long, conflicts_with = "skills")]
    pub all: bool,
    /// Add this tag to every adopted skill (repeatable).
    #[arg(long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
    /// Set the author of every adopted skill.
    #[arg(long, value_name = "NAME")]
    pub author: Option<String>,
    /// Set the mode of every adopted skill (`default-on`, `default-off`,
    /// `required`).
    #[arg(long, value_name = "MODE")]
    pub mode: Option<String>,
    /// Per-skill frontmatter edits as JSON, `{"<new name>": {fields}}`
    /// (the web UI's review step).
    #[arg(long, value_name = "JSON", hide = true)]
    pub edits: Option<String>,
    /// Put back an adopted skill's original that `lo sync` moved aside.
    #[arg(long, value_name = "NAME", conflicts_with_all = ["into", "dir", "skills", "all", "paths"])]
    pub restore: Option<String>,
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
    /// A kebab-case name to adopt it under, when its own isn't.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_name: Option<String>,
    /// Why it's probably already shared: adopted into a source and waiting
    /// for it to ship, or a source already ships a skill of this name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub already: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// `applies_to.role`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// False when its SKILL.md frontmatter can't be read.
    #[serde(skip)]
    readable: bool,
}

/// One adopted skill.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Adopted {
    pub name: String,
    /// Its name where it was found, when it was renamed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed_from: Option<String>,
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
    /// `--restore`: the original put back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored: Option<crate::handover::Restored>,
    pub warnings: Vec<String>,
}

impl Report for AdoptReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if let Some(r) = &self.restored {
            writeln!(out, "Put {} back at {}.", r.name, r.path)?;
            for l in &r.links {
                writeln!(out, "  re-linked {l}")?;
            }
            return Ok(());
        }
        let Some(source) = &self.source else {
            if self.candidates.is_empty() {
                return writeln!(out, "No skills found that Loadout doesn't already manage.");
            }
            writeln!(out, "Skills you could adopt into a source:")?;
            for c in &self.candidates {
                let note = c
                    .problem
                    .as_deref()
                    .or(c.already.as_deref())
                    .map(|p| format!("  [{p}]"));
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
                let renamed = a
                    .renamed_from
                    .as_deref()
                    .map(|r| format!(" (was {r})"))
                    .unwrap_or_default();
                writeln!(out, "  {:<28} from {}{renamed}", a.id, a.from)?;
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
                "Once your source ships them, `lo sync` moves your originals aside (`lo adopt --restore <name>` puts one back)."
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
    if let Some(name) = &args.restore {
        let restored = crate::handover::restore(&ctx.paths, name, &mut warnings)?;
        ctx.emit(&AdoptReport {
            candidates: Vec::new(),
            source: None,
            adopted: Vec::new(),
            skipped: Vec::new(),
            working_clone: false,
            restored: Some(restored),
            warnings,
        })?;
        return Ok(exit::OK);
    }
    let mut candidates = if args.paths.is_empty() {
        installed(ctx, &mut warnings)?
    } else {
        let mut found = Vec::new();
        for p in &args.paths {
            found.extend(in_folder(p, "path", &BTreeSet::new(), &mut warnings)?);
        }
        found
    };
    mark_already(ctx, &mut candidates)?;
    let edits = edits(&args)?;
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
            restored: None,
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
    let mut handovers = crate::handover::AdoptedFile::load(&ctx.paths.adopted_file())?;
    for (c, new_name) in picked {
        if args.all
            && let Some(why) = &c.already
        {
            skipped.push(Skipped {
                name: c.name.clone(),
                reason: why.clone(),
            });
            continue;
        }
        let name = new_name.unwrap_or_else(|| c.name.clone());
        let mut fields = edits.get(&name).cloned().unwrap_or_default();
        shared_fields(&args, &c, &mut fields);
        match copy_into(ctx, &dest, &source, &c, &name, &fields, &mut warnings) {
            Ok(a) => {
                // Only a tool's own folder is handed over; a folder given
                // as a path (a project's, say) is the user's business.
                if c.found_in != "path"
                    && let Ok(hash) = crate::handover::folder_hash(Path::new(&c.path))
                {
                    handovers.record(crate::handover::Adoption {
                        name: name.clone(),
                        source: source.clone(),
                        original: PathBuf::from(&c.path),
                        content_hash: hash,
                        backup: None,
                        links: Vec::new(),
                    });
                }
                adopted.push(a);
            }
            Err(e) => skipped.push(Skipped {
                name: c.name.clone(),
                reason: format!("{e:#}"),
            }),
        }
    }
    if !adopted.is_empty() {
        handovers.save(&ctx.paths.adopted_file())?;
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
        restored: None,
        warnings,
    })?;
    Ok(code)
}

/// The selection, each with its new name if renamed: `--all`, `--skill`,
/// or a multi-select in a terminal.
fn pick(
    ctx: &Ctx,
    args: &AdoptArgs,
    candidates: &[Candidate],
) -> Result<Vec<(Candidate, Option<String>)>> {
    if args.all {
        return Ok(candidates.iter().map(|c| (c.clone(), None)).collect());
    }
    if !args.skills.is_empty() {
        let mut out = Vec::new();
        for spec in &args.skills {
            let (name, new) = match spec.split_once('=') {
                Some((n, new)) => (n, Some(new.trim().to_owned())),
                None => (spec.as_str(), None),
            };
            let c = candidates
                .iter()
                .find(|c| c.name == name)
                .with_context(|| {
                    format!("no skill named {name:?} found (`lo adopt` lists them)")
                })?;
            out.push((c.clone(), new.filter(|n| *n != c.name)));
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
            match c.problem.as_ref().or(c.already.as_ref()) {
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
    Ok(chosen
        .into_iter()
        .map(|i| (candidates[i].clone(), None))
        .collect())
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
    let readable = meta.is_ok();
    let (meta, mut problem) = match meta {
        Ok(m) => (m, None),
        Err(e) => {
            warnings.push(format!("{}: {e:#}", dir.join(SKILL_FILE).display()));
            (
                ItemMeta::default(),
                Some("SKILL.md frontmatter can't be read".to_owned()),
            )
        }
    };
    let mut suggested_name = None;
    if problem.is_none() && validate_item_name(&name).is_err() {
        problem = Some("name isn't kebab-case; adopt it under a new name".to_owned());
        suggested_name = Some(kebab(&name)).filter(|n| validate_item_name(n).is_ok());
    }
    let block = meta.loadout.unwrap_or_default();
    Candidate {
        name,
        description: meta.description,
        path: dir.display().to_string(),
        found_in: found_in.to_owned(),
        problem,
        suggested_name,
        already: None,
        tags: block.tags,
        author: block.author,
        mode: block
            .mode
            .and_then(|m| serde_json::to_value(m).ok())
            .and_then(|v| v.as_str().map(str::to_owned)),
        roles: block.applies_to.get("role").cloned().unwrap_or_default(),
        readable,
    }
}

/// Copies `c` into the source at `dest` as `name`, then applies `fields`
/// (see [`crate::edit::apply_fields`]) to its SKILL.md.
fn copy_into(
    ctx: &Ctx,
    dest: &Path,
    source: &str,
    c: &Candidate,
    name: &str,
    fields: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> Result<Adopted> {
    if !c.readable {
        bail!("SKILL.md frontmatter can't be read");
    }
    if validate_item_name(name).is_err() {
        if name == c.name {
            bail!("name isn't kebab-case; adopt it under a new name (`--skill {name}=<new-name>`)");
        }
        bail!("{name:?} isn't kebab-case");
    }
    let key = ItemKey::new(ItemKind::Skill, name)?;
    work::check_editable(ctx, dest, &key)?;
    let (item_rel, _, _) = work::item_path(dest, &key)?;
    let item_dir = work::join(dest, &item_rel);
    if item_dir.exists() {
        bail!("{source} already has {item_rel}");
    }
    let (mut files, tree_warnings) = crate::scan::read_tree(Path::new(&c.path))?;
    warnings.extend(
        tree_warnings
            .into_iter()
            .map(|w| format!("{}: {w}", c.name)),
    );
    // Edit SKILL.md before writing anything, so a bad edit writes nothing.
    if let Some(main) = files.iter_mut().find(|f| f.path == SKILL_FILE) {
        let mut text = String::from_utf8(main.contents.clone()).context("SKILL.md isn't UTF-8")?;
        if name != c.name {
            text = fe::set_top_key(&text, "name", Some(name));
        }
        if !fields.is_empty() {
            text = crate::edit::apply_fields(&text, &Value::Object(fields.clone()))
                .with_context(|| format!("editing {name}"))?;
        }
        main.contents = text.into_bytes();
    }
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
        name: name.to_owned(),
        renamed_from: (name != c.name).then(|| c.name.clone()),
        id: format!("{source}:skill/{name}"),
        from: c.path.clone(),
        path: item_dir.display().to_string(),
        files: names,
    })
}

/// Marks candidates that are probably shared already: adopted and waiting
/// for their source to ship them, or shipped by a source under the same
/// name.
fn mark_already(ctx: &Ctx, candidates: &mut [Candidate]) -> Result<()> {
    let adopted = crate::handover::AdoptedFile::load(&ctx.paths.adopted_file())?;
    let resolved = crate::state::Resolved::load(&ctx.paths.resolved_file())?.unwrap_or_default();
    for c in candidates.iter_mut() {
        if let Some(a) = adopted.pending(Path::new(&c.path)) {
            c.already = Some(format!(
                "adopted into {} as skill/{}; moved aside once {} ships it",
                a.source, a.name, a.source
            ));
        } else if let Some(i) = resolved
            .items
            .iter()
            .find(|i| i.kind == ItemKind::Skill && i.name == c.name)
        {
            c.already = Some(format!("{} already ships skill/{}", i.source, i.name));
        }
    }
    Ok(())
}

/// `--edits`: the frontmatter fields to set on each adopted skill, by its
/// new name.
fn edits(args: &AdoptArgs) -> Result<BTreeMap<String, Map<String, Value>>> {
    match &args.edits {
        Some(json) => {
            serde_json::from_str(json).context("--edits must be a JSON object of objects")
        }
        None => Ok(BTreeMap::new()),
    }
}

/// Adds `--tag` to the skill's tags, and fills in `--author` and `--mode`
/// where its own edits don't set them.
fn shared_fields(args: &AdoptArgs, c: &Candidate, fields: &mut Map<String, Value>) {
    if !args.tags.is_empty() {
        let mut tags: Vec<String> = match fields.get("tags").and_then(Value::as_array) {
            Some(a) => a
                .iter()
                .filter_map(|t| t.as_str().map(str::to_owned))
                .collect(),
            None => c.tags.clone(),
        };
        for t in &args.tags {
            if !tags.contains(t) {
                tags.push(t.clone());
            }
        }
        fields.insert("tags".into(), Value::from(tags));
    }
    if let Some(a) = &args.author {
        fields
            .entry("author")
            .or_insert_with(|| Value::from(a.clone()));
    }
    if let Some(m) = &args.mode {
        fields
            .entry("mode")
            .or_insert_with(|| Value::from(m.clone()));
    }
}

/// `My_Skill Name` → `my-skill-name`.
fn kebab(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn kebab_cases_folder_names() {
        assert_eq!(super::kebab("Bad_Name"), "bad-name");
        assert_eq!(super::kebab("  My  Skill!! "), "my-skill");
        assert_eq!(super::kebab("___"), "");
    }
}

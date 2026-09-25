//! `lo template [list|show|use]`: templates published by
//! higher sources, turned into real skills in a source you can push to.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use loadout_git::Git;
use loadout_model::frontmatter::Document;
use loadout_model::manifest::MANIFEST_FILE;
use loadout_model::template::TemplateVar;
use loadout_model::{ItemKey, ItemKind, ItemMeta, ManifestDoc};
use schemars::JsonSchema;
use serde::Serialize;

use crate::ctx::{Ctx, Report};
use crate::exit;
use crate::scan::{SKILL_FILE, ScannedTemplate, scan_templates};
use crate::sources::repo_dir;
use crate::state::Resolved;
use crate::work;

#[derive(Debug, Args)]
pub struct TemplateArgs {
    #[command(subcommand)]
    pub command: Option<TemplateCommand>,
}

#[derive(Debug, Subcommand)]
pub enum TemplateCommand {
    /// Templates in your sources (the default).
    List(ListArgs),
    /// A template's variables and files.
    Show(ShowArgs),
    /// Create a skill from a template in a source you can push to.
    Use(UseArgs),
}

#[derive(Debug, Default, Args)]
pub struct ListArgs {
    /// Also list templates from every company config-listed source you can read.
    #[arg(long)]
    pub all_sources: bool,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// `source:template/name`, `template/name` or just the name.
    pub template: String,
    /// Also look in every company config-listed source you can read.
    #[arg(long)]
    pub all_sources: bool,
}

#[derive(Debug, Args)]
pub struct UseArgs {
    /// `source:template/name`, `template/name` or just the name.
    pub template: String,
    /// The source (its `LOADOUT.md` name) to add the skill to, in its working
    /// clone; then `lo export-source <source>` opens a pull request.
    #[arg(
        long,
        value_name = "SOURCE",
        conflicts_with = "dir",
        required_unless_present = "dir"
    )]
    pub into: Option<String>,
    /// Or: a local checkout of a source repo to write the skill into.
    #[arg(long, value_name = "PATH")]
    pub dir: Option<PathBuf>,
    /// Name of the new skill [default: the template's name].
    #[arg(long)]
    pub name: Option<String>,
    /// A variable's value (repeatable): `--set team=checkout`.
    #[arg(long = "set", value_name = "NAME=VALUE", value_parser = parse_kv)]
    pub set: Vec<(String, String)>,
    /// Also look in every company config-listed source you can read.
    #[arg(long)]
    pub all_sources: bool,
}

fn parse_kv(s: &str) -> Result<(String, String), String> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| format!("expected NAME=VALUE, got {s:?}"))?;
    Ok((k.trim().to_owned(), v.to_owned()))
}

/// One template, as listed.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TemplateEntry {
    /// `source:template/name`.
    pub id: String,
    pub source: String,
    pub name: String,
    /// What the template is for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The description the generated skill gets (may use variables).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub variables: Vec<TemplateVar>,
    /// Files, relative to the template directory.
    pub files: Vec<String>,
    /// From a source you're not subscribed to (`--all-sources`).
    pub not_subscribed: bool,
}

/// `--json` output of `lo template list`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TemplateListReport {
    pub templates: Vec<TemplateEntry>,
    pub warnings: Vec<String>,
}

impl Report for TemplateListReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if self.templates.is_empty() {
            return writeln!(
                out,
                "No templates in your sources. A source publishes them under templates/<name>/SKILL.md."
            );
        }
        let width = self.templates.iter().map(|t| t.id.len()).max().unwrap_or(0);
        for t in &self.templates {
            let vars: Vec<&str> = t.variables.iter().map(|v| v.name.as_str()).collect();
            writeln!(
                out,
                "{:<width$}  {}{}",
                t.id,
                t.description
                    .as_deref()
                    .or(t.skill_description.as_deref())
                    .unwrap_or(""),
                if t.not_subscribed {
                    "  (not subscribed)"
                } else {
                    ""
                }
            )?;
            if !vars.is_empty() {
                writeln!(out, "{:<width$}  variables: {}", "", vars.join(", "))?;
            }
        }
        writeln!(
            out,
            "Use one: `lo template use <id> --into <your-source> --set name=value`"
        )
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// `--json` output of `lo template show`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TemplateShowReport {
    #[serde(flatten)]
    pub template: TemplateEntry,
    /// The template's `SKILL.md`.
    pub text: String,
}

impl Report for TemplateShowReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        let t = &self.template;
        writeln!(out, "{}", t.id)?;
        if let Some(d) = &t.description {
            writeln!(out, "{d}")?;
        }
        if !t.variables.is_empty() {
            writeln!(out, "\nVariables:")?;
            for v in &t.variables {
                let default = v
                    .default
                    .as_deref()
                    .map(|d| format!(" (default: {d})"))
                    .unwrap_or_else(|| " (required)".to_owned());
                writeln!(
                    out,
                    "  {}{default}  {}",
                    v.name,
                    v.description.as_deref().unwrap_or("")
                )?;
            }
        }
        writeln!(out, "\nFiles: {}\n", t.files.join(", "))?;
        write!(out, "{}", self.text)
    }
}

/// `--json` output of `lo template use`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TemplateUseReport {
    pub template: String,
    /// `source:skill/name` of the new skill.
    pub id: String,
    /// The source it was written to.
    pub source: String,
    /// Directory of the new skill.
    pub path: String,
    /// Files written, relative to the skill directory.
    pub files: Vec<String>,
    /// The values used.
    pub values: BTreeMap<String, String>,
    /// Written to the source's working clone (`--into`), ready for
    /// `lo export-source`.
    pub working_clone: bool,
    pub warnings: Vec<String>,
}

impl Report for TemplateUseReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        writeln!(out, "Created {} from {}", self.id, self.template)?;
        writeln!(out, "  in {}", self.path)?;
        if self.working_clone {
            writeln!(
                out,
                "Review it, then open a pull request: lo export-source {} -m \"Add {}\"",
                self.source,
                self.id.rsplit('/').next().unwrap_or_default()
            )
        } else {
            writeln!(out, "Review it, commit and push.")
        }
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

pub fn run(ctx: &Ctx, args: TemplateArgs) -> Result<u8> {
    match args
        .command
        .unwrap_or(TemplateCommand::List(ListArgs::default()))
    {
        TemplateCommand::List(a) => {
            let (found, warnings) = collect(ctx, a.all_sources)?;
            ctx.emit(&TemplateListReport {
                templates: found.iter().map(|(t, n)| entry(t, *n)).collect(),
                warnings,
            })?;
        }
        TemplateCommand::Show(a) => {
            let (found, _) = collect(ctx, a.all_sources)?;
            let (t, n) = find(&found, &a.template)?;
            let text = main_text(t)?;
            ctx.emit(&TemplateShowReport {
                template: entry(t, n),
                text,
            })?;
        }
        TemplateCommand::Use(a) => {
            let report = use_template(ctx, a)?;
            ctx.emit(&report)?;
        }
    }
    Ok(exit::OK)
}

/// A template and whether its source is one you're not subscribed to.
type Found = (ScannedTemplate, bool);

/// Templates from subscribed sources (at their applied commits), plus
/// unsubscribed company config sources with `all`; and warnings.
fn collect(ctx: &Ctx, all: bool) -> Result<(Vec<Found>, Vec<String>)> {
    let Some(resolved) = Resolved::load(&ctx.paths.resolved_file())? else {
        bail!("nothing synced yet; run `lo sync`");
    };
    let mut warnings = Vec::new();
    let mut dirs: Vec<(PathBuf, bool)> = resolved
        .sources
        .iter()
        .map(|s| (repo_dir(&ctx.paths, &s.url), false))
        .collect();
    if all {
        dirs.extend(
            crate::commands::search::unsubscribed_company_config_sources(ctx, &mut warnings)?
                .into_iter()
                .map(|(d, _)| (d, true)),
        );
    }
    let mut out = Vec::new();
    for (dir, not_subscribed) in dirs {
        match scan_templates(&dir) {
            Ok((ts, w)) => {
                warnings.extend(w);
                out.extend(ts.into_iter().map(|t| (t, not_subscribed)));
            }
            Err(e) => warnings.push(format!("{}: {e:#}", dir.display())),
        }
    }
    out.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    Ok((out, warnings))
}

fn find<'a>(found: &'a [Found], wanted: &str) -> Result<(&'a ScannedTemplate, bool)> {
    let short = wanted.strip_prefix("template/").unwrap_or(wanted);
    let matches: Vec<&Found> = found
        .iter()
        .filter(|(t, _)| t.id == wanted || t.name == short)
        .collect();
    match matches.as_slice() {
        [(t, n)] => Ok((t, *n)),
        [] => bail!(
            "no template {wanted:?} in your sources (see `lo template list`, or add --all-sources)"
        ),
        many => bail!(
            "{wanted:?} is ambiguous: {}",
            many.iter()
                .map(|(t, _)| t.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn entry(t: &ScannedTemplate, not_subscribed: bool) -> TemplateEntry {
    TemplateEntry {
        id: t.id.clone(),
        source: t.source.clone(),
        name: t.name.clone(),
        description: t.front.template.description.clone(),
        skill_description: t.front.description.clone(),
        tags: t
            .front
            .loadout
            .as_ref()
            .map(|g| g.tags.clone())
            .unwrap_or_default(),
        variables: t.front.template.variables.clone(),
        files: t.files.iter().map(|f| f.path.clone()).collect(),
        not_subscribed,
    }
}

fn main_text(t: &ScannedTemplate) -> Result<String> {
    let main = t
        .files
        .iter()
        .find(|f| f.path == SKILL_FILE)
        .context("template has no SKILL.md")?;
    Ok(String::from_utf8(main.contents.clone())?)
}

fn use_template(ctx: &Ctx, args: UseArgs) -> Result<TemplateUseReport> {
    let (found, _) = collect(ctx, args.all_sources)?;
    let (t, _) = find(&found, &args.template)?;
    let name = args.name.clone().unwrap_or_else(|| t.name.clone());
    let key = ItemKey::new(ItemKind::Skill, &name)?;

    // Values: --set, then defaults, then prompts (in a terminal).
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in &args.set {
        if !t.front.template.variables.iter().any(|d| &d.name == k) {
            bail!("{} has no variable {k:?}", t.id);
        }
        values.insert(k.clone(), v.clone());
    }
    let interactive = !ctx.json && std::io::stdin().is_terminal();
    let mut missing = Vec::new();
    for v in &t.front.template.variables {
        if values.contains_key(&v.name) {
            continue;
        }
        if interactive {
            let mut input = dialoguer::Input::<String>::new().with_prompt(format!(
                "{}{}",
                v.name,
                v.description
                    .as_deref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default()
            ));
            if let Some(d) = &v.default {
                input = input.default(d.clone());
            }
            values.insert(v.name.clone(), input.interact_text()?);
        } else if let Some(d) = &v.default {
            values.insert(v.name.clone(), d.clone());
        } else {
            missing.push(v.name.clone());
        }
    }
    if !missing.is_empty() {
        bail!(
            "{} needs values for: {} (use --set name=value)",
            t.id,
            missing.join(", ")
        );
    }

    // Destination.
    let git = Git::new();
    let (dest, working_clone) = match (&args.into, &args.dir) {
        (Some(source), _) => {
            let w = work::open(ctx, &git, source)?;
            (w.dir, true)
        }
        (None, Some(dir)) => (dir.clone(), false),
        (None, None) => bail!("say where: --into <source> or --dir <path>"),
    };
    let manifest_text = std::fs::read_to_string(dest.join(MANIFEST_FILE)).with_context(|| {
        format!(
            "{} is not a source ({MANIFEST_FILE} missing)",
            dest.display()
        )
    })?;
    let dest_name = ManifestDoc::parse(&manifest_text)?.manifest.name;
    work::check_editable(ctx, &dest, &key)?;
    let (item_rel, _, _) = work::item_path(&dest, &key)?;
    let item_dir = work::join(&dest, &item_rel);
    if item_dir.exists() {
        bail!("{} already exists; pick another --name", item_dir.display());
    }

    let commit = git.head(&t.root).map(|c| c.to_string()).unwrap_or_default();
    let provenance = if commit.is_empty() {
        t.id.clone()
    } else {
        format!("{}@{}", t.id, &commit[..commit.len().min(12)])
    };
    let mut warnings = Vec::new();
    let mut files = Vec::new();
    let mut rendered: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    for f in &t.files {
        let bytes = match std::str::from_utf8(&f.contents) {
            Ok(text) => {
                let text = if f.path == SKILL_FILE {
                    loadout_core::template::instantiate_frontmatter(text, &name, &provenance)
                        .map_err(anyhow::Error::msg)?
                } else {
                    text.to_owned()
                };
                let (out, unknown) = loadout_core::template::render(&text, &values);
                for u in unknown {
                    warnings.push(format!("{}: {{{{{u}}}}} has no value; left as is", f.path));
                }
                if f.path == SKILL_FILE {
                    let _: ItemMeta = Document::split(&out)
                        .and_then(|d| d.parse())
                        .context(
                            "the filled-in SKILL.md frontmatter isn't valid YAML (a value may need quoting in the template)",
                        )?;
                }
                out.into_bytes()
            }
            Err(_) => f.contents.clone(),
        };
        rendered.push((work::join(&item_dir, &f.path), bytes));
        files.push(f.path.clone());
    }
    for (path, bytes) in rendered {
        std::fs::create_dir_all(path.parent().expect("has parent"))?;
        std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(TemplateUseReport {
        template: t.id.clone(),
        id: format!("{dest_name}:skill/{name}"),
        source: dest_name,
        path: item_dir.display().to_string(),
        files,
        values,
        working_clone,
        warnings,
    })
}

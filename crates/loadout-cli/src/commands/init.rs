//! `lo init <company-config-url>` — first-time setup: fetch the
//! company config, discover membership, let the user adjust groups and targets,
//! save, and run the first sync.
//!
//! `lo init --new-source [DIR]` / `--new-company [DIR]` create a source
//! (catalog) or a company config instead; with no arguments in a
//! terminal, `lo init` asks which of these you want.

use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use loadout_git::Git;
use loadout_members::{Basis, Discovery, GroupStatus};
use loadout_model::config::normalize_url;
use loadout_model::id::validate_source_name;
use loadout_model::layer::USER_LAYER;
use loadout_model::{Config, Membership};
use loadout_targets::TargetTable;
use schemars::JsonSchema;
use serde::Serialize;

use crate::commands::new_source::{self, NewSourceReport, ScaffoldArgs};
use crate::commands::subscribe::checked_url;
use crate::commands::sync::exit_code;
use crate::ctx::{Ctx, Report};
use crate::engine::{self, ApplyReport, Fetch, Options, enabled_targets};
use crate::membership;

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Git URL (or local path) of your company config repo (not used with
    /// `--project`, `--new-source` or `--new-company`).
    pub company_config: Option<String>,
    /// Accept discovered groups and detected tools without prompting.
    #[arg(long)]
    pub non_interactive: bool,
    /// Create a new source (a catalog of skills, MCP servers, …) in DIR
    /// [default: the current directory] instead of joining a company config.
    #[arg(long, visible_alias = "catalog", value_name = "DIR", num_args = 0..=1,
          default_missing_value = ".", conflicts_with = "company_config")]
    pub new_source: Option<PathBuf>,
    /// Create a new company config in DIR [default: the current directory].
    #[arg(long, value_name = "DIR", num_args = 0..=1, default_missing_value = ".",
          conflicts_with_all = ["company_config", "new_source"])]
    pub new_company: Option<PathBuf>,
    /// New source: default layer of its items (team, org, squad, role, user, …).
    #[arg(long)]
    pub layer: Option<String>,
    /// New source: default group [default: the name; your user name for `--layer user`].
    #[arg(long)]
    pub group: Option<String>,
    /// New source: its id (kebab-case) [default: the directory name].
    #[arg(long)]
    pub name: Option<String>,
    /// New source: a higher source it builds on (repeatable).
    #[arg(long, value_name = "URL")]
    pub upstream: Vec<String>,
    /// New company config: the company name.
    #[arg(long)]
    pub company: Option<String>,
    /// New company config: your layer names and ranks, e.g.
    /// `company=0,division=10,team=20,user=50` (higher rank wins ties).
    #[arg(long, value_name = "NAME=RANK,...", requires = "new_company")]
    pub layers: Option<String>,
    /// New source: `git init` it (if needed) and commit the scaffold.
    #[arg(long)]
    pub git_init: bool,
    /// New source: subscribe to it and sync, to try it right away (implies
    /// `--git-init`).
    #[arg(long)]
    pub subscribe: bool,
}

/// `--json` output of `lo init --new-source` / `--new-company`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct InitCreateReport {
    pub created: NewSourceReport,
    /// The scaffold was committed to a Git repo.
    pub committed: bool,
    /// The sync after subscribing (`--subscribe`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<ApplyReport>,
}

impl Report for InitCreateReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        match &self.sync {
            None if !self.committed => self.created.human(out),
            _ => {
                let what = if self.created.company_config {
                    "company_config"
                } else {
                    "source"
                };
                writeln!(
                    out,
                    "Created and committed {what} {:?} ({}:{}) in {}.",
                    self.created.name, self.created.layer, self.created.group, self.created.dir
                )?;
                match &self.sync {
                    Some(sync) => {
                        writeln!(
                            out,
                            "Subscribed; edit it, commit, and `lo sync` picks it up."
                        )?;
                        sync.human(out)
                    }
                    None if self.created.company_config => writeln!(
                        out,
                        "Next: push it to your Git host, add groups under company.groups, and have people run `lo init <url>`."
                    ),
                    None => writeln!(
                        out,
                        "Next: push it to your Git host and list it in your company config (or `lo subscribe <url>`)."
                    ),
                }
            }
        }
    }

    fn warnings(&self) -> &[String] {
        self.sync.as_ref().map_or(&[], |s| &s.warnings)
    }
}

/// `--json` output of `lo init`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct InitReport {
    pub company_config: String,
    pub company: String,
    pub groups: Vec<GroupStatus>,
    pub targets: Vec<String>,
    pub sync: ApplyReport,
}

impl Report for InitReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        writeln!(
            out,
            "Company config: {} ({})",
            self.company_config, self.company
        )?;
        writeln!(out, "Your groups:")?;
        for u in self.groups.iter().filter(|u| u.member) {
            writeln!(out, "  {}:{}", u.layer, u.group)?;
        }
        if self.targets.is_empty() {
            writeln!(out, "Tools: none enabled")?;
        } else {
            writeln!(out, "Tools: {}", self.targets.join(", "))?;
        }
        self.sync.human(out)
    }

    fn warnings(&self) -> &[String] {
        &self.sync.warnings
    }
}

pub fn run(ctx: &Ctx, args: InitArgs) -> Result<u8> {
    if ctx.paths.project.is_some() {
        if args.new_source.is_some() || args.new_company.is_some() {
            bail!(
                "`--project` sets up this repo's project layer; drop it to create a source or company config"
            );
        }
        if args.company_config.is_some() {
            bail!(
                "a project layer doesn't take a company config; run `lo init --project` without a URL"
            );
        }
        return crate::commands::project::init(ctx);
    }
    let interactive = !args.non_interactive && !ctx.json && std::io::stdin().is_terminal();
    let mut args = args;
    if args.company_config.is_none() && args.new_source.is_none() && args.new_company.is_none() {
        if !interactive {
            bail!(
                "what should init do? `lo init <company-config-url>` joins your company's setup; `lo init --new-source [dir]` creates a source (catalog); `lo init --new-company [dir]` creates a company config; `lo init --project` sets up this repo"
            );
        }
        pick_mode(&mut args)?;
    }
    if args.new_source.is_some() || args.new_company.is_some() {
        return create(ctx, args);
    }
    let Some(company_config) = &args.company_config else {
        unreachable!("pick_mode sets one of company_config/new_source/new_company");
    };
    let url = checked_url(company_config)?;
    let config = ctx.load_config()?;
    if let Some(existing) = &config.company_config
        && normalize_url(existing) != normalize_url(&url)
    {
        bail!(
            "already set up with company config {existing}; remove `company_config` from config.toml to switch"
        );
    }
    let git = Git::new();
    let loaded = membership::load_company_config(ctx, &git, &url, true)?;
    let mut discovery = membership::run_discovery(&loaded.company_config, &config);
    let mut choices = config.membership.clone();
    if interactive {
        pick_groups(&mut discovery, &mut choices)?;
    }

    let table = TargetTable::load(&ctx.paths.user_targets_dir())?;
    let targets: Vec<String> = match &config.targets.enabled {
        Some(ids) if !interactive => ids.clone(),
        _ => {
            let probe = Config {
                targets: Default::default(),
                ..config.clone()
            };
            let (detected, _) = enabled_targets(&table, &probe, &ctx.paths.home);
            let detected: Vec<String> = detected.iter().map(|t| t.id.clone()).collect();
            if interactive {
                pick_targets(&table, &detected)?
            } else {
                detected
            }
        }
    };

    let mut doc = ctx.load_config_doc()?;
    doc.set_company_config(&url);
    doc.set_membership(&choices);
    if !targets.is_empty() || interactive {
        doc.set_targets_enabled(&targets);
    }
    ctx.save_config(&doc)?;
    membership::save_discovery(ctx, &url, &discovery)?;

    let config = ctx.load_config()?;
    let company = loaded.company_config.name.clone();
    let sync = engine::apply(
        ctx,
        &config,
        Options {
            company_config: Some(loaded),
            ..Options::new(Fetch::Remote)
        },
    )?;
    let mut sync = sync;
    sync.warnings.splice(0..0, discovery.warnings.clone());
    let report = InitReport {
        company_config: url,
        company,
        groups: discovery.groups,
        targets,
        sync,
    };
    ctx.emit(&report)?;
    Ok(exit_code(&report.sync))
}

/// `lo init` with no arguments in a terminal: what do you want to do?
fn pick_mode(args: &mut InitArgs) -> Result<()> {
    let choices = [
        "Join my company's setup (I have a company config URL)",
        "Create a source (catalog) for my team, squad, product or role",
        "Create a personal source for my own skills, building on my team's",
        "Create a company config",
    ];
    let pick = dialoguer::Select::new()
        .with_prompt("What would you like to set up?")
        .items(choices)
        .default(0)
        .interact()
        .context("reading your choice")?;
    let prompt = |label: &str, default: Option<String>| -> Result<String> {
        let mut input = dialoguer::Input::<String>::new().with_prompt(label);
        if let Some(d) = default {
            input = input.default(d);
        }
        input.interact_text().context("reading input")
    };
    // Kebab-case ids are checked as they're typed, not after every prompt.
    let prompt_id = |label: &str, default: String| -> Result<String> {
        let mut input = dialoguer::Input::<String>::new().with_prompt(label);
        if !default.is_empty() {
            input = input.default(default);
        }
        input
            .validate_with(|v: &String| -> Result<(), String> {
                validate_source_name(v.trim()).map_err(|e| e.to_string())
            })
            .interact_text()
            .map(|v| v.trim().to_owned())
            .context("reading input")
    };
    match pick {
        0 => args.company_config = Some(prompt("Company config URL", None)?),
        1 | 2 => {
            let personal = pick == 2;
            let name = prompt_id(
                "Source name (kebab-case)",
                if personal {
                    format!("{}-skills", user_name())
                } else {
                    "team-skills".to_owned()
                },
            )?;
            args.new_source = Some(PathBuf::from(prompt("Directory", Some(name.clone()))?));
            args.name = Some(name.clone());
            args.layer = Some(if personal {
                USER_LAYER.to_owned()
            } else {
                let layers = [
                    "team",
                    "squad",
                    "org",
                    "product",
                    "role",
                    "company",
                    "another layer my company config defines…",
                ];
                let i = dialoguer::Select::new()
                    .with_prompt("Layer of its items")
                    .items(layers)
                    .default(0)
                    .interact()
                    .context("reading the layer")?;
                if i == layers.len() - 1 {
                    prompt_id(
                        "Layer name (as in your company config's layers)",
                        String::new(),
                    )?
                } else {
                    layers[i].to_owned()
                }
            });
            if !personal {
                args.group = Some(prompt_id("Group (e.g. payments-dev)", name.clone())?);
            }
            let ups = prompt(
                "Higher sources it builds on (URLs, comma-separated; empty for none)",
                Some(String::new()),
            )?;
            args.upstream = ups
                .split(',')
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(str::to_owned)
                .collect();
            args.git_init = dialoguer::Confirm::new()
                .with_prompt("Create a Git repo and commit it?")
                .default(true)
                .interact()
                .context("reading your answer")?;
            if args.git_init {
                args.subscribe = dialoguer::Confirm::new()
                    .with_prompt("Subscribe to it now so you can try it?")
                    .default(personal)
                    .interact()
                    .context("reading your answer")?;
            }
        }
        _ => {
            let company = prompt_id("Company name (kebab-case)", "acme".to_owned())?;
            let name = format!("{company}-config");
            args.new_company = Some(PathBuf::from(prompt("Directory", Some(name.clone()))?));
            args.name = Some(name);
            args.company = Some(company);
            let layers = dialoguer::Input::<String>::new()
                .with_prompt(
                    "Layers and ranks (name=rank, comma-separated; the higher rank wins when two layers provide the same item)",
                )
                .default(new_source::DEFAULT_LAYERS.to_owned())
                .validate_with(|v: &String| -> Result<(), String> {
                    new_source::parse_layers(v)
                        .map(drop)
                        .map_err(|e| e.to_string())
                })
                .interact_text()
                .context("reading the layers")?;
            if layers != new_source::DEFAULT_LAYERS {
                args.layers = Some(layers);
            }
            args.git_init = dialoguer::Confirm::new()
                .with_prompt("Create a Git repo and commit it?")
                .default(true)
                .interact()
                .context("reading your answer")?;
        }
    }
    Ok(())
}

/// The login name, for personal sources: `$USER` / `%USERNAME%`, in
/// kebab-case.
pub fn user_name() -> String {
    let raw = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "me".to_owned());
    let mut out = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() { "me".to_owned() } else { out }
}

/// `--new-source` / `--new-company`: scaffold, optionally commit, subscribe
/// and sync.
fn create(ctx: &Ctx, args: InitArgs) -> Result<u8> {
    let company_config = args.new_company.is_some();
    let dir = args
        .new_company
        .clone()
        .or(args.new_source.clone())
        .expect("create() needs a directory");
    let personal = args.layer.as_deref() == Some(USER_LAYER);
    let scaffold = ScaffoldArgs {
        layer: args.layer.clone(),
        group: args.group.clone().or_else(|| personal.then(user_name)),
        name: args.name.clone(),
        upstream: args.upstream.clone(),
        company_config,
        company: args.company.clone(),
        layers: args.layers.clone(),
    };
    // Bad names fail before anything is created.
    new_source::check(&dir, &scaffold)?;
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let created = new_source::scaffold(&dir, &scaffold)?;
    let git_init = args.git_init || args.subscribe;
    if git_init {
        commit_scaffold(
            &dir,
            if company_config {
                "Set up company config"
            } else {
                "Set up source"
            },
        )?;
    }
    let sync = if args.subscribe && !company_config {
        crate::commands::subscribe::add_subscription(
            ctx,
            crate::commands::subscribe::SubscribeArgs {
                url: dir.display().to_string(),
                layer: None,
                group: None,
                priority: 0,
                git_ref: None,
                dry_run: false,
            },
        )?;
        let config = ctx.load_config()?;
        Some(engine::apply(ctx, &config, Options::new(Fetch::Remote))?)
    } else {
        None
    };
    let code = sync.as_ref().map_or(crate::exit::OK, exit_code);
    ctx.emit(&InitCreateReport {
        created,
        committed: git_init,
        sync,
    })?;
    Ok(code)
}

/// `git init` (unless `dir` is already in a repo), add everything, commit.
fn commit_scaffold(dir: &Path, message: &str) -> Result<()> {
    let git = Git::new();
    if git.run(Some(dir), ["rev-parse", "--git-dir"]).is_err() {
        git.run(Some(dir), ["init", "--quiet", "--initial-branch", "main"])
            .context("git init")?;
    }
    git.run(Some(dir), ["add", "--all"]).context("git add")?;
    git.run(Some(dir), ["commit", "--quiet", "-m", message])
        .context("committing the scaffold (is your Git user.name/user.email set?)")?;
    Ok(())
}

/// Shows discovered groups and lets the user add or remove them before
/// saving. Changes become `joined`/`left` choices.
fn pick_groups(d: &mut Discovery, choices: &mut Membership) -> Result<()> {
    let groups: Vec<&GroupStatus> = d
        .groups
        .iter()
        .filter(|u| u.basis != Basis::Company)
        .collect();
    if groups.is_empty() {
        return Ok(());
    }
    let labels: Vec<String> = groups
        .iter()
        .map(|u| format!("{}:{}", u.layer, u.group))
        .collect();
    let defaults: Vec<bool> = groups.iter().map(|u| u.member).collect();
    let picked = dialoguer::MultiSelect::new()
        .with_prompt("Your groups (space toggles, enter confirms)")
        .items(&labels)
        .defaults(&defaults)
        .interact()
        .context("reading group selection")?;
    let changes: Vec<(String, bool)> = labels
        .iter()
        .enumerate()
        .filter(|(i, _)| picked.contains(i) != defaults[*i])
        .map(|(i, id)| (id.clone(), picked.contains(&i)))
        .collect();
    for (id, member) in changes {
        choices.joined.retain(|u| *u != id);
        choices.left.retain(|u| *u != id);
        if member {
            choices.joined.push(id.clone());
        } else {
            choices.left.push(id.clone());
        }
        if let Some(u) = d
            .groups
            .iter_mut()
            .find(|u| format!("{}:{}", u.layer, u.group) == id)
        {
            u.member = member;
            u.basis = if member { Basis::Joined } else { Basis::Left };
        }
    }
    d.profile = d
        .groups
        .iter()
        .filter(|u| u.member)
        .map(|u| (u.layer.as_str(), u.group.as_str()))
        .collect();
    Ok(())
}

fn pick_targets(table: &TargetTable, detected: &[String]) -> Result<Vec<String>> {
    let all: Vec<&str> = table.iter().map(|t| t.id.as_str()).collect();
    let labels: Vec<String> = table
        .iter()
        .map(|t| {
            if detected.contains(&t.id) {
                format!("{} (detected)", t.display)
            } else {
                t.display.clone()
            }
        })
        .collect();
    let defaults: Vec<bool> = all
        .iter()
        .map(|id| detected.iter().any(|d| d == id))
        .collect();
    let picked = dialoguer::MultiSelect::new()
        .with_prompt("Install into which tools?")
        .items(&labels)
        .defaults(&defaults)
        .interact()
        .context("reading tool selection")?;
    Ok(picked.into_iter().map(|i| all[i].to_owned()).collect())
}

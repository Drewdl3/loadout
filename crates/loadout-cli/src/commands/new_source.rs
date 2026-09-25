//! `lo new-source [<dir>] [--layer s] [--group u] [--name n]` — scaffold a
//! source repo: `LOADOUT.md`, item directories, an example skill
//! and a CI workflow that audits the repo.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::Args;
use loadout_model::id::validate_source_name;
use loadout_model::manifest::MANIFEST_FILE;
use schemars::JsonSchema;
use serde::Serialize;

use crate::ctx::{Ctx, Report};
use crate::exit;

#[derive(Debug, Args)]
pub struct NewSourceArgs {
    /// Directory to create or fill (default: the current directory).
    pub dir: Option<PathBuf>,
    #[command(flatten)]
    pub scaffold: ScaffoldArgs,
}

/// What to put in a new source's `LOADOUT.md` (shared with `lo init`).
#[derive(Debug, Clone, Default, Args)]
pub struct ScaffoldArgs {
    /// Default layer of the repo's items (e.g. team, org, squad, role,
    /// user for a personal source) [default: team; company for a company config].
    #[arg(long)]
    pub layer: Option<String>,
    /// Default group (e.g. payments-dev) [default: the source name].
    #[arg(long)]
    pub group: Option<String>,
    /// Source id (kebab-case; default: the directory name).
    #[arg(long)]
    pub name: Option<String>,
    /// A higher source this one builds on (repeatable): subscribers get its
    /// items too. `../x` is relative to this repo's URL.
    #[arg(long, value_name = "URL")]
    pub upstream: Vec<String>,
    /// Make it a company config: layers, groups and policy.
    #[arg(long)]
    pub company_config: bool,
    /// The company name in a company config [default: the group].
    #[arg(long, requires = "company_config")]
    pub company: Option<String>,
    /// A company config's own layer names and ranks, e.g.
    /// `company=0,division=10,team=20,user=50`; the higher rank wins when
    /// two layers provide the same item [default: company=0, org=10,
    /// team=20, product=25, squad=30, project=35, role=40, user=50].
    #[arg(long, requires = "company_config", value_name = "NAME=RANK,...")]
    pub layers: Option<String>,
}

/// The layers a new company config gets when `--layers` isn't given.
pub const DEFAULT_LAYERS: &str =
    "company=0,org=10,team=20,product=25,squad=30,project=35,role=40,user=50";

/// Parses `name=rank,...` (kebab-case names, unique names and ranks).
pub fn parse_layers(text: &str) -> Result<Vec<(String, i64)>> {
    let mut out: Vec<(String, i64)> = Vec::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let Some((name, rank)) = part.split_once('=') else {
            bail!("layer {part:?} needs a rank: name=rank, e.g. team=20");
        };
        let (name, rank) = (name.trim(), rank.trim());
        validate_source_name(name)
            .map_err(|_| anyhow::anyhow!("layer names are kebab-case, got {name:?}"))?;
        let rank: i64 = rank.parse().map_err(|_| {
            anyhow::anyhow!("rank of {name:?} must be a whole number, got {rank:?}")
        })?;
        if let Some((other, _)) = out.iter().find(|(n, r)| n == name || *r == rank) {
            bail!("layers {other:?} and {name:?} clash: each needs its own name and rank");
        }
        out.push((name.to_owned(), rank));
    }
    if !out.iter().any(|(n, _)| n == "company") {
        bail!("layers must include `company` (the layer everyone is in), e.g. company=0");
    }
    out.sort_by_key(|(_, r)| *r);
    Ok(out)
}

/// `--json` output of `lo new-source`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct NewSourceReport {
    pub name: String,
    pub dir: String,
    pub layer: String,
    pub group: String,
    /// True for a company config.
    pub company_config: bool,
    /// Files written, relative to `dir`.
    pub created: Vec<String>,
}

impl Report for NewSourceReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        let what = if self.company_config {
            "company_config"
        } else {
            "source"
        };
        writeln!(
            out,
            "Created {what} {:?} ({}:{}) in {}:",
            self.name, self.layer, self.group, self.dir
        )?;
        for f in &self.created {
            writeln!(out, "  {f}")?;
        }
        writeln!(out, "Next:")?;
        writeln!(
            out,
            "  git -C {:?} init && git -C {:?} add -A && git -C {:?} commit -m \"Set up {what}\"",
            self.dir, self.dir, self.dir
        )?;
        writeln!(out, "  push it to your Git host, then:")?;
        if self.company_config {
            writeln!(
                out,
                "  add groups and their repos under company.groups in LOADOUT.md, and have people run `lo init <url>`"
            )
        } else {
            writeln!(
                out,
                "  list it in your company config's groups (or `lo subscribe <url>`); try it locally with `lo subscribe {:?}`",
                self.dir
            )
        }
    }
}

fn kebab(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

pub fn run(ctx: &Ctx, args: NewSourceArgs) -> Result<u8> {
    let dir = match args.dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    };
    let report = scaffold(&dir, &args.scaffold)?;
    ctx.emit(&report)?;
    Ok(exit::OK)
}

/// The source name `dir` would get by default.
pub fn default_name(dir: &Path) -> String {
    let base = std::fs::canonicalize(dir)
        .unwrap_or_else(|_| std::path::absolute(dir).unwrap_or_else(|_| dir.to_owned()))
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    kebab(&base)
}

/// The ids a new source gets.
pub struct Names {
    pub name: String,
    pub layer: String,
    pub group: String,
}

/// Validates the names a scaffold would use, before anything is written.
pub fn check(dir: &Path, args: &ScaffoldArgs) -> Result<Names> {
    let name = args.name.clone().unwrap_or_else(|| default_name(dir));
    validate_source_name(&name)?;
    let company_config_layers = if args.company_config {
        parse_layers(args.layers.as_deref().unwrap_or(DEFAULT_LAYERS))?
    } else {
        Vec::new()
    };
    let layer = args.layer.clone().unwrap_or_else(|| {
        if args.company_config {
            "company"
        } else {
            "team"
        }
        .to_owned()
    });
    if layer.trim().is_empty() || layer.contains(char::is_whitespace) {
        bail!("layer must be a single word, got {layer:?}");
    }
    // A company config's group is the company.
    let group = args
        .group
        .clone()
        .or_else(|| args.company_config.then(|| args.company.clone()).flatten())
        .unwrap_or_else(|| name.clone());
    if args.company_config {
        let company = args.company.as_deref().unwrap_or(&group);
        validate_source_name(company)
            .map_err(|_| anyhow::anyhow!("company must be kebab-case, got {company:?}"))?;
        if !company_config_layers.iter().any(|(n, _)| *n == layer) {
            bail!("the company config's own layer {layer:?} must be one of its layers (--layers)");
        }
    }
    Ok(Names { name, layer, group })
}

/// Writes a new source (or company config) into `dir`; never overwrites a file.
pub fn scaffold(dir: &Path, args: &ScaffoldArgs) -> Result<NewSourceReport> {
    if dir.join(MANIFEST_FILE).exists() {
        bail!(
            "{} already has a {MANIFEST_FILE}; nothing was changed",
            dir.display()
        );
    }
    let Names { name, layer, group } = check(dir, args)?;
    let upstream = if args.upstream.is_empty() {
        String::new()
    } else {
        let mut u = "upstream:\n".to_owned();
        for url in &args.upstream {
            u.push_str(&format!("  - {url:?}\n"));
        }
        u
    };

    let manifest = if args.company_config {
        let company = args.company.clone().unwrap_or_else(|| group.clone());
        let layers = parse_layers(args.layers.as_deref().unwrap_or(DEFAULT_LAYERS))?;
        company_config_manifest(&name, &layer, &group, &company, &upstream, &layers)
    } else {
        format!(
            "---\nloadout: 1\nname: {name}\nlayer: {layer}\ngroup: {group}\ndescription: Agent configuration for {layer} {group}.\ndefaults:\n  mode: default-on\n{upstream}---\n\n# {name}\n\nSkills (`skills/<name>/SKILL.md`), MCP servers (`mcp/<name>.md`) and other\nagent configuration for the `{layer}:{group}` group, distributed by\n[Loadout](https://github.com/Drewdl3/loadout).\n\n- Put secret **references** in MCP definitions (`secret://jira/token`),\n  never values.\n- Items can override the defaults above in a `loadout:` frontmatter block\n  (`mode: required | default-on | default-off`, `applies_to`, `locked`, …).\n- `upstream:` lists higher sources this one builds on; subscribers get\n  their items too.\n"
        )
    };
    let files: Vec<(&str, String)> = vec![
        (MANIFEST_FILE, manifest),
        ("AGENTS.md", agents_md(&name, &layer, &group, args.company_config)),
        (
            "skills/example/SKILL.md",
            format!(
                "---\nname: example\ndescription: An example skill from {name}. Replace me.\nloadout:\n  mode: default-off\n---\n\n# Example\n\nDescribe when the agent should use this skill and what to do.\n"
            ),
        ),
        (
            "mcp/README.md",
            "# MCP servers\n\nOne file per server, e.g. `jira.md`:\n\n```markdown\n---\nname: jira\ndescription: Jira for our tickets.\ncommand: npx\nargs: [\"-y\", \"@acme/jira-mcp@1.2.3\"]\nenv:\n  JIRA_TOKEN: \"secret://jira/token\"\n---\n```\n".to_owned(),
        ),
        (
            ".github/workflows/loadout-audit.yml",
            "# Audit this repo's agent configuration on every change (Loadout).\nname: lo audit\non:\n  pull_request:\n  push:\n    branches: [main]\njobs:\n  audit:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - uses: Drewdl3/loadout/shims/github-action@main\n        with:\n          args: audit --path .\n".to_owned(),
        ),
    ];
    // What we write must parse.
    loadout_model::ManifestDoc::parse(&files[0].1)?;
    let mut created = Vec::new();
    for (rel, text) in &files {
        let path = dir.join(rel);
        if path.exists() {
            continue;
        }
        std::fs::create_dir_all(path.parent().expect("has parent"))?;
        std::fs::write(&path, text)?;
        created.push((*rel).to_owned());
    }
    Ok(NewSourceReport {
        name,
        dir: dir.display().to_string(),
        layer,
        group,
        company_config: args.company_config,
        created,
    })
}

/// Instructions for AI agents working in the new repo (docs/agents.md, short).
fn agents_md(name: &str, layer: &str, group: &str, company_config: bool) -> String {
    let what = if company_config {
        "the company **company config**: the root source that also lists layers, groups and policy under `company:` in `LOADOUT.md`"
    } else {
        "an Loadout **source**"
    };
    let company_config_notes = if company_config {
        "\n## The company config\n\n- `company.layers`: layer names and ranks. The higher rank wins when two\n  sources provide the same item. `company` is required.\n- `company.groups`: one entry per group: `layer`, `name`, `sources: [urls]`,\n  `membership:` (`github_team`, `gitlab_group`, `env`, `exec`,\n  `repo_access`, `manual`, `any`/`all`).\n- `company.policy`: `auto_apply` (layers whose updates skip review),\n  `allow_manual_sources`, `require_signed`.\n- Items here have layer `company`: everyone gets them. Use\n  `mode: required` + `locked: true` only for what must not be turned off.\n"
    } else {
        ""
    };
    format!(
        r#"# AGENTS.md — {name}

This repo is {what}. [Loadout](https://github.com/Drewdl3/loadout)
installs its items for everyone in `{layer}:{group}`. Full guide for agents:
https://github.com/Drewdl3/loadout/blob/main/docs/agents.md

## Layout

| Path | Item |
|---|---|
| `LOADOUT.md` | Manifest (`name: {name}`, `layer: {layer}`, `group: {group}`, `defaults`, `upstream`). Keep `name` stable. |
| `skills/<name>/SKILL.md` | A skill, plus any files it uses. `name` = the directory name. |
| `mcp/<name>.md` | An MCP server: `command`/`args`/`env`, or `url`/`headers`. |
| `agents/<name>.md` | A subagent. |
| `extras/<type>/<name>.md` | `commands`, `rules` or `prompts`. |
| `templates/<name>/SKILL.md` | A template others fill in (`{{{{variable}}}}` + a `template:` block). Never installed. |

Any item may have a `loadout:` block in its frontmatter: `mode` (`required`,
`default-on`, `default-off`), `tags`, `applies_to: {{ role: [...] }}`, `locked`,
`overridable`, `targets`, or a different `layer`/`group`.
{company_config_notes}
## Writing items

- Names are kebab-case: lowercase letters, digits, single `-`.
- A skill's `description` says what it does **and when to use it**; agents
  decide from it whether to load the skill.
- MCP secrets are references (`secret://jira/token`, `env://VAR`), never
  values. Pin package versions (`@acme/jira-mcp@1.2.3`).
- To start from a template in a higher source: `lo template list`, then
  `lo template use <template> --dir . --set key=value`.

## Before you commit

```sh
lo audit --path .              # formats, prompt injection, secrets
git add -A && git commit -m "…"   # Loadout reads commits, not the working tree
lo subscribe . --dry-run       # what subscribers would get, upstreams included
```

Changes reach people through this repo's pull requests. Don't push to the
default branch without review.
"#
    )
}

fn company_config_manifest(
    name: &str,
    layer: &str,
    group: &str,
    company: &str,
    upstream: &str,
    layers: &[(String, i64)],
) -> String {
    let layers: String = layers
        .iter()
        .map(|(n, r)| format!("    - {{ name: {n}, rank: {r} }}\n"))
        .collect();
    let example_layer = if layers.contains("name: team,") {
        "team"
    } else {
        layer
    };
    format!(
        r#"---
loadout: 1
name: {name}
layer: {layer}
group: {group}
description: {company}'s Loadout company_config.
{upstream}company:
  name: {company}
  # Rank = specificity: when two layers provide the same item, the higher
  # rank wins unless a lower one marks it `locked: true`.
  # Layer names are yours to choose; sources use them in `layer:`.
  layers:
{layers}  # Every group and the repos it owns. Membership rules: github_team,
  # gitlab_group, env, exec, repo_access, manual, any/all. For example:
  #   - layer: {example_layer}
  #     name: payments-dev
  #     sources: ["https://git.example.com/{company}/payments-skills"]
  #     membership: {{ github_team: "{company}/payments-eng" }}
  groups: []
  policy:
    auto_apply: [company]        # updates from these layers apply without review
    allow_manual_sources: true   # may people subscribe to repos not listed here?
---

# {company} company_config

Company-wide agent configuration and the registry of every group's repo.
New here? Run `lo init <this repo's URL>`.
"#
    )
}

# Loadout for AI agents

This page is for an AI agent (Claude Code, Codex, Cursor, Pi, Bob, …) that
has to **use** the `lo` CLI or **write** Loadout content: skills, MCP
servers, subagents, templates, sources and company configs. Humans may find it a
useful cheat sheet too.

Two skills carry the short versions into any tool:
[`loadout`](../shims/claude-code/skills/loadout/SKILL.md) (using the CLI) and
[`loadout-authoring`](../shims/claude-code/skills/loadout-authoring/SKILL.md)
(writing sources). A team's source (or a company config) can ship both, so
every agent of everyone subscribed gets them ([docs/any-agent.md](any-agent.md)). Every repo made by
`lo new-source` or `lo init --new-source` also contains an `AGENTS.md`
with that repo's names already filled in.

## The model

```
you (user:jane) ──lo init <link>──▶ source checkout-squad   (squad, rank 30)
                                     └─ upstream ▶ payments-skills (team, rank 20)
                                          └─ upstream ▶ eng-skills (org, rank 10)
                ──subscribe──────▶ source beta-skills       (beta, rank 27: its own layer)
                ──personal───────▶ source jane-skills       (user, rank 50)

optional: company config (company:acme): layers, groups → sources, membership, policy
```

- A **source** is a Git repo with a `LOADOUT.md` at its top. It holds **items**:
  `skill/<name>`, `mcp/<name>`, `agent/<name>`, `plugin/<name>`,
  `extra/<type>.<name>`.
- People **subscribe** to sources, usually their team's (`lo init <url>`),
  and get each source's **upstreams** (`upstream:`) too. Subscribing makes
  you a member of the source's group.
- **Layers** rank sources. The ranks merge, later winning: built-in
  defaults, the `layers:` of subscribed sources, the company config's
  `company.layers`, the user's `[layers]` (`lo layers`). Each subscriber can
  also place a source at another layer or rank (`lo source set`).
- An optional **company config** is a source with a `company:` block. It
  lists every **group** (`team:payments-dev`), the sources that group owns,
  and how membership is discovered (`github_team`, `env`, …), plus policy.
- A person's **profile** is the set of groups they belong to. `lo sync`
  collects the items of every source they get. When two sources provide
  the same `kind/name`, the **higher rank wins** (more specific beats more
  general), unless a more general one set `locked: true` or
  `overridable: false`. Locks are compared at **declared** ranks (before any
  local re-rank), so re-ranking never changes which lock wins.
- The winners are linked or rendered into each AI tool's own directories
  (`~/.claude/skills`, `~/.claude.json`, `~/.cursor/…`). Those copies belong to
  Loadout: the next sync overwrites any edits made there.

## Using the CLI

Always pass `--json` and parse stdout. Add `--exit-zero` when a non-zero exit
would abort your tool call; the outcome is in the JSON either way. Each
command's JSON Schema is in [`schemas/cli/`](../schemas/cli/).

| Exit code | Meaning |
|---|---|
| 0 | OK |
| 1 | Error (message on stderr) |
| 2 | Changes are waiting for review (`lo status`, `lo diff`, `lo approve`) |
| 3 | An audit finding blocked an update |
| 4 | Unresolved conflicts (`lo why`, `lo prefer`) |

| Task | Command |
|---|---|
| Where does an item come from; why this version | `lo why skill/write-spec` |
| What is installed | `lo list --enabled [--kind skill]` |
| Find something | `lo search "specs" [--kind skill] [--tag t] [--role r] [--all-sources]` |
| Turn one on or off | `lo enable <source:kind/name>` / `lo disable …` |
| Pick between two equal-rank sources | `lo prefer <source:kind/name>` |
| Layers in effect, with where each rank came from | `lo layers` |
| Re-rank a layer or a source for the user | `lo layers set beta=27`, `lo source set <url-or-name> --rank 35 [--label L] [--layer l] [--priority n]` |
| Connect to a link someone shared (source or company config) | `lo init <url> --non-interactive` |
| What a repo would bring | `lo subscribe <url-or-path> --dry-run` |
| Pending updates | `lo status`, `lo diff [<source>]` |
| Health | `lo doctor` |

Only run `lo approve`, `lo export-source` or anything that pushes when
the user has asked for it. Never print, log or ask for secret values:
`lo secrets set <name>` prompts the user directly.

## Writing a source

### Create one

```sh
lo init --new-source checkout-squad --layer squad --group checkout \
  --upstream https://git.example.com/acme/payments-skills --git-init
```

`--layer` is any layer: a built-in one (`company`, `org`, `team`,
`product`, `squad`, `project`, `role`), one your company config defines, or
one of your own with `--rank` to suggest where it ranks
(`--layer beta --rank 27`). `--layer user` makes a personal source. Names
are kebab-case. This writes:

```
checkout-squad/
├── LOADOUT.md                 # the manifest (required)
├── AGENTS.md                # this guide, short, with the repo's names filled in
├── skills/example/SKILL.md  # skills/<name>/SKILL.md, plus any files the skill uses
├── mcp/README.md            # mcp/<name>.md, one per server
└── .github/workflows/loadout-audit.yml
```

Other directories you may add: `agents/<name>.md` (subagents),
`extras/<type>/<name>.md` (`commands`, `rules`, `prompts`),
`plugins/<name>/PLUGIN.md` (Claude Code plugins) and
`templates/<name>/SKILL.md` (templates, never installed).

Skills that already exist somewhere (in `~/.claude/skills`, a project's
`.claude/skills`, a folder) can be copied in rather than rewritten:
`lo adopt` lists the ones Loadout doesn't manage, and
`lo adopt [<folder>] --dir <checkout> --skill <name>` (or `--into <source>`
for a working clone) copies them with their supporting files.

### `LOADOUT.md`

```markdown
---
loadout: 1
name: checkout-squad          # the source id: kebab-case, stable
layer: squad                  # default layer of every item here
group: checkout                # default group
description: Skills for the Checkout squad.
owners: ["@acme/checkout"]    # shown in `lo why`
defaults:
  mode: default-on            # required | default-on | default-off
layers:                       # suggested ranks for layers of your own (optional; must be above company's)
  - { name: pods, rank: 32 }
upstream:                     # higher sources subscribers also get
  - https://git.example.com/acme/payments-skills
  - url: ../beta-skills         # relative to this repo's URL
    label: Beta (payments)      # where it sits for this source's subscribers:
    layer: product              #   its items' layer, their own included,
    rank: 27                    #   or an exact rank,
    priority: 10                #   and the tie-breaker (all optional)
nested: [pods]                # folders holding more LOADOUT.md files (optional)
---

# Checkout squad

Free text for humans; `lo info checkout-squad` shows it.
```

### Nested sources: one repo, several manifests

A repo can hold a team's items and a folder per squad. The root
`LOADOUT.md` lists the folders under `nested:`, and every `LOADOUT.md`
found below them (at any depth, skipping hidden folders and symlinks) is a
source of its own:

```
payments-team/
├── LOADOUT.md            # name: payments-team, layer: team, nested: [squads]
├── skills/…
└── squads/
    ├── beaver/
    │   ├── LOADOUT.md    # name: beaver-squad, layer: squad, group: beaver
    │   └── skills/…
    └── alpha/
        ├── LOADOUT.md    # name: alpha-squad, layer: squad, group: alpha
        └── skills/…
```

- A nested source has its own `name`, `layer`, `group`, `defaults` and
  `paths` (relative to its folder); its item ids use its name
  (`beaver-squad:skill/dam`).
- It shares the repo's fetch, commit, review and `loadout.lock` entry, so
  the repo's layers are combined: an update applies without review only
  if every one of them is in `policy.auto_apply`, and a commit must be
  signed if any of them is in `policy.require_signed`. Review lists a
  nested source's items by full id (`changed: beaver-squad:skill/dam`).
- Subscribing to the repo (or a company config group listing it) gets all
  of them, but you only get a nested source's items if you're in its group:
  through the company config (a group `squad:beaver` with a membership
  rule, or `lo join squad:beaver`), or without one, `squad = ["beaver"]`
  under `[profile]` in `config.toml`. Subscribing makes you a member of the
  root's group only.
- A nested `LOADOUT.md` may list `nested:` itself. `upstream:` and
  `company:` are only read from the repo's root `LOADOUT.md`.
- Names must be unique across everything you subscribe to; a nested
  source whose name is taken is skipped with a warning, as is one whose
  `LOADOUT.md` doesn't parse.
- Edits in a working copy (`lo export-source beaver-squad`, **Edit** in
  `lo ui`) go to the source's folder; exporting proposes every edit in that
  repo's working copy.

### Skills: `skills/<name>/SKILL.md`

The standard `SKILL.md` format. Loadout reads an optional `loadout:` block
and leaves every other key alone.

```markdown
---
name: write-spec
description: Write a feature spec in the Checkout format. Use when asked for a spec, PRD or design doc.
loadout:
  mode: default-on            # overrides defaults.mode
  tags: [specs, writing]
  applies_to: { role: [developer, product-manager] }   # AND across axes, OR within one
  locked: false               # true: lower layers can't disable or replace it
  overridable: true           # false: lower layers can't replace it
  targets: [claude-code]      # only these tools (default: all)
  author: Ada Lovelace        # who wrote it (shown in the UI, `lo info`, searchable)
  co_authors: [Grace Hopper]  # others who helped (`coAuthors` also accepted)
---

# Writing a spec
…
```

Keep `name` equal to the directory name (the directory wins if they differ). The `description` decides when
agents load the skill, so say what it does **and** when to use it.

### MCP servers: `mcp/<name>.md`

```markdown
---
name: jira
description: Jira, with Acme's ticket conventions.
command: npx
args: ["-y", "@acme/jira-mcp@1.2.3"]     # pin versions
env:
  JIRA_BASE_URL: "https://acme.atlassian.net"
  JIRA_TOKEN: "secret://jira/token"      # a reference, never a value
---
```

HTTP: `url: https://mcp.example.com/x` and
`headers: { Authorization: "Bearer secret://x/token" }`. References:
`secret://name` (the company config's resolver chain), `env://VAR`, `op://…`,
`vault://path#field`, `sops://path#key`, `keychain://service/account`. A literal token in a source fails the audit.

### Subagents and extras

`agents/<name>.md` uses the tool's own subagent frontmatter (`name`,
`description`, `tools`) plus an optional `loadout:` block.
`extras/<type>/<name>.md` goes to each tool that has a place for that type:
`commands` are slash commands (Claude Code, OpenCode, Bob), `rules` go to
Bob, `prompts` to Pi. `lo targets --json` lists what each tool takes.

### Templates: `templates/<name>/SKILL.md`

A template is a skill others copy into their own source and fill in. It is
never installed itself.

```markdown
---
name: pr-workflow
description: How {{team}} works with pull requests. Use when opening or reviewing a PR.
template:
  description: A pull request workflow skill each squad fills in.
  variables:
    - { name: team, description: "Your squad, e.g. Checkout" }
    - { name: reviewers, description: "Approvals needed", default: "2" }
---

# Pull requests in {{team}}
Approvals required: {{reviewers}}.
```

To use one: `lo template list`, `lo template show pr-workflow`, then
`lo template use pr-workflow --into <source> --set team=Checkout`, or
`--dir <checkout>` for a local clone. The copy records
`loadout: { from_template: "<source>:template/pr-workflow@<commit>" }`.

### Where a source sits for a subscriber

The subscriber's `config.toml` decides, overriding `upstream:` entries:

```toml
[layers]
beta = 27                   # re-rank a layer (lo layers set beta=27)

[[source]]
url = "https://git.example.com/acme/beta-skills"
label = "Beta (payments)"   # display name in lo why, lo list, lo ui
layer = "product"           # rank every item at this layer, item-level layer: included
rank = 27                   # or exactly this rank
priority = 10               # tie-break within a rank
```

A `[[source]]` for a URL the company config lists only places it (unless the
company config sets `policy.allow_local_ranks: false`); one for an upstream's
URL subscribes to that upstream directly. Placements apply to the repo's
root source, not its nested ones. Locks ignore all of this.

### A company config (optional)

Add one once several teams use Loadout and you want groups discovered
automatically, or company-wide policy.

```sh
lo init --new-company acme-config --company acme \
  --layers company=0,org=10,team=20,squad=30,user=50 --git-init
```

Register each group under `company.groups`:

```yaml
groups:
  - layer: squad
    name: checkout
    sources: ["https://git.example.com/acme/checkout-squad"]
    membership: { github_team: "acme/checkout" }   # or env, exec, repo_access, manual, any/all
```

`company.policy` takes `auto_apply` (layers whose updates skip review),
`allow_manual_sources`, `allow_local_ranks` (may people re-rank the company
config's layers and sources; locks hold either way) and `require_signed`.

## Check your work

Run these from the source repo before you commit:

```sh
lo audit --path .                 # formats, prompt injection, secrets, dangerous commands
git add -A && git commit -m "…"      # sources are read from commits, not the working tree
lo subscribe . --dry-run          # what subscribers would get, upstreams included
```

To see it installed without touching the user's real setup, use a sandbox:
`LOADOUT_HOME`, `LOADOUT_CONFIG_DIR` and `HOME` pointing at a temporary
directory, then `lo subscribe <path>` and `lo sync`.

## Rules

- Change the source repo, never the installed copies in `~/.claude` and
  similar directories.
- Keep secrets out of Git. Use references.
- Don't lower a `locked` or `required` item from a lower source; it is
  refused, and `lo why` shows who locked it.
- Changes reach other people through the repo's normal review. Open a pull
  request (`lo export-source <source>`) and never push to the default
  branch of someone else's source.
- JSON Schemas for every format are in [`schemas/`](../schemas/).

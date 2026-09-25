---
name: loadout-authoring
description: How to write Loadout content — skills, MCP servers, subagents, templates, sources (LOADOUT.md) and company configs — in a Git repo that Loadout distributes. Use when creating or editing a source repo, adding a skill or MCP server for a team, making or filling in a template, or setting up a company config.
---

# Writing Loadout sources

A **source** is a Git repo with `LOADOUT.md` at its top. Its items are
installed for everyone in its group. Full guide:
https://github.com/Drewdl3/loadout/blob/main/docs/agents.md

## Layout

| Path | Item |
|---|---|
| `LOADOUT.md` | Manifest: `loadout: 1`, `name` (kebab-case), `layer`, `group`, `description`, `defaults: { mode }`, `upstream: [urls]` |
| `skills/<name>/SKILL.md` | Skill (+ any files it uses); `name` = directory name |
| `mcp/<name>.md` | MCP server: `command`/`args`/`env`, or `url`/`headers` |
| `agents/<name>.md` | Subagent |
| `extras/<type>/<name>.md` | `commands`, `rules`, `prompts` |
| `templates/<name>/SKILL.md` | Template with `{{variables}}` and a `template:` block; never installed |

Optional `loadout:` block in any item's frontmatter: `mode` (`required`,
`default-on`, `default-off`), `tags`, `applies_to: { role: [...] }`,
`locked`, `overridable`, `targets`, `author`, `co_authors`, `layer`/`group`
overrides.

## Steps

1. New repo: `lo init --new-source <dir> --layer <layer> --group <group> [--upstream <url>] --git-init`.
   The layer must exist in the company config's `company.layers`; `user` is personal.
2. Write the item. A skill's `description` says what it does **and when to
   use it**. MCP secrets are references (`secret://jira/token`,
   `env://VAR`), never values. Pin package versions.
3. Or start from something that exists:
   - A skill you already use (in `~/.claude/skills`, a project's `.claude/skills`, …):
     `lo adopt [<folder>]` lists them, `lo adopt [<folder>] --dir <checkout> --skill <name>` copies one in.
   - A template: `lo template list`, `lo template show <t>`,
     `lo template use <t> --into <source> --set key=value` (or `--dir <checkout>`).
4. Check: `lo audit --path .`, commit, then `lo subscribe . --dry-run`
   (sources are read from commits).
5. Publish through review: push a branch and open a pull request, or
   `lo export-source <source> -m "…"` when the user asks. Never push to
   another team's default branch.

## Rules

- Edit the source repo, never the installed copies (`~/.claude/skills`, …).
- A lower source can't disable or replace a `locked`/`required` item; check with `lo why <kind>/<name>`.
- Kebab-case for source, group, layer and item names.

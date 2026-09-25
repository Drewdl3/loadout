---
name: loadout
description: How to use the Loadout `lo` CLI, which installs this machine's skills, MCP servers and agent configuration from the company's Git repos. Use when the user asks where a skill or MCP server comes from, why it is (or isn't) installed, to turn one on or off, to find one, or about pending configuration updates.
---

# Using Loadout (`lo`)

Loadout installs agent configuration (skills, MCP servers, subagents,
rules) from repos owned by the company, org, team, product and role groups the
user belongs to. Always pass `--json` and read the result; add `--exit-zero`
so status codes don't abort your command.

| Question | Command |
|---|---|
| Where does a skill/server come from, why this version? | `lo why <kind>/<name>` (e.g. `skill/write-spec`, `mcp/jira`) |
| What is installed? | `lo list --enabled` (add `--kind skill` or `--kind mcp`) |
| Anything waiting for review? | `lo status`, then `lo diff [<source>]` |
| Turn an item on/off | `lo enable <source:kind/name>` / `lo disable …` (required or locked items can't be disabled) |
| Find a skill or server | `lo search "<words>" [--kind skill] [--tag t] [--role r]` (add `--all-sources` for groups the user isn't in) |
| What would a source bring? | `lo subscribe <url> --dry-run` |
| Adapt a template for the user's team | `lo template list`, then `lo template use <t> --into <source> --set k=v` (and `lo export-source <source>` to open a PR, only when asked) |
| Which groups am I in? | `lo profile` |
| Something broken? | `lo doctor` |
| Secrets for MCP servers | `lo secrets check` (never print secret values) |

Rules:

- Don't edit skills, MCP entries or rules that Loadout installed (in
  `~/.claude`, `~/.bob`, `~/.cursor`, …) or its data directory by hand — the
  next sync overwrites them. Change the source repo or use `lo enable/disable`.
- `lo approve` applies changes someone else authored; only run it when the
  user explicitly asks, after showing `lo diff`.
- Never ask for or print secret values; `lo secrets set <name>` prompts the
  user directly.

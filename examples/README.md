# Example: Acme

A small, complete Loadout setup for a fictional company, **Acme**. Every
directory here is what one Git repo would contain; all URLs use
`git.example.com`.

| Repo | Layer : group | What it shows |
|---|---|---|
| [`acme-config`](acme-config/LOADOUT.md) | `company:acme` | The **company config**: layers and ranks, every group with its repo and membership rule, policy, secret chain. Also company-wide items: a `required` + `locked` skill and a Jira MCP server whose token is `secret://jira/token`. |
| [`eng-skills`](eng-skills/LOADOUT.md) | `org:eng` | Org-wide skills, `default-on` from `LOADOUT.md` `defaults:`; one `default-off` skill; a subagent (`agents/code-reviewer.md`). |
| [`payments-skills`](payments-skills/LOADOUT.md) | `team:payments-dev` | A team **override** of Engineering's `write-spec`; a skill limited with `applies_to: { role: [developer] }`; an HTTP MCP server with an `env://` header; a slash command (`extras/commands/reconcile.md`). |
| [`billing-config`](billing-config/LOADOUT.md) | `product:billing` | An opt-in product group (`membership: { manual: true }`). |
| [`pm-presets`](pm-presets/LOADOUT.md) | `role:product-manager` | A role that cuts across teams. |

Membership rules use environment variables (`ACME_ORG`, `ACME_TEAM`,
`ACME_ROLE`) so the example runs offline; a real company config would typically use
`{ github_team: "acme/engineering" }`.

The company config also ships the `loadout` and `loadout-authoring` skills (so every AI tool knows the CLI and how to write sources),
and `eng-skills` publishes a `pr-workflow` **template** that squads adapt.

## Onboarding story

`./examples/onboard.sh` (with `LOADOUT=…` as below) plays out
[`docs/onboarding.md`](../docs/onboarding.md):

- a squad lead creates the Checkout squad's source on top of Payments, fills
  in the PR template, and registers the squad in the company config;
- a new developer joins and adds a personal layer;
- the squad pushes an update, which the developer reviews and approves.

## Try it

```sh
cargo build && LOADOUT=$PWD/target/debug/lo ./examples/try.sh
```

The script copies each directory into a local Git repo inside a temporary
sandbox, points the company config at them, and runs `lo init` for a Payments
developer with a fake home directory — your real `~/.claude` is never
touched. You should see:

- `code-review-standards` (company, locked), `write-spec` (the **Payments**
  version: team outranks org), `pci-checklist` (developers only);
- no `prd-template` (product managers only) and no `incident-runbook`
  (`default-off`);
- in the sandbox's `~/.claude.json`, `jira` launched as
  `lo mcp-run acme-config:mcp/jira` — the token is resolved when Claude
  Code starts the server, from `JIRA_TOKEN` or the keychain
  (`lo secrets set jira/token`), and never written to disk.

`lo why skill/write-spec` explains the override:

```text
skill/write-spec — enabled (default)
winner: payments-skills:skill/write-spec (most specific layer)
candidates:
  ✓ payments-skills:skill/write-spec  team:payments-dev (rank 20, priority 0) — winner
  ✗ eng-skills:skill/write-spec       org:eng (rank 10, priority 0) — outranked by payments-skills:skill/write-spec
targets:
  claude-code: ~/.claude/skills/write-spec (link)
```

Then try `lo join product:billing`, `lo enable
eng-skills:skill/incident-runbook`, or `lo disable
acme-config:skill/code-review-standards` (refused: it is required and
locked). As a product manager:
`ACME_ROLE=product-manager ACME_ORG= ACME_TEAM= ./examples/try.sh`.

The company config sets `policy.auto_apply: [company]`: company changes land on the
next sync; changes to the other repos wait for `lo diff` / `lo
approve`.

This tree is also an end-to-end test: `crates/loadout-cli/tests/examples.rs`.

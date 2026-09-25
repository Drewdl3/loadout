# Harness shims

Thin integrations; all logic stays in `lo`.

| Shim | What it does | Install |
|---|---|---|
| [`claude-code/`](claude-code) | Claude Code plugin: `/lo <command>`, `lo sync --if-stale` on session start, a `loadout` skill | `/plugin marketplace add Drewdl3/loadout`, then `/plugin install loadout@loadout` |
| [`pi/`](pi) | Pi extension: `/lo <command>`, sync on session start | copy `pi/loadout` to `~/.pi/agent/extensions/` |
| [`github-action/`](github-action) | CI: install `lo`, `sync --locked` | `uses: Drewdl3/loadout/shims/github-action@main` |

Other tools, including IBM Bob (a built-in target), need no shim. See
[`docs/any-agent.md`](../docs/any-agent.md) for session-start hooks, your
own target files for any tool, and distributing the `loadout` skill through
your company config.

# AI tools

Loadout installs your items into each AI tool you use, in that tool's own
format. It calls these tools *targets*.

## Built-in tools

| Tool | Skills | Subagents | Tool connections (MCP) | Other |
|---|---|---|---|---|
| Claude Code | `~/.claude/skills` | `~/.claude/agents` | `~/.claude.json` | commands; plugins |
| Pi | `~/.pi/agent/skills` | | `~/.pi/agent/mcp.json` | prompts |
| Codex | `~/.agents/skills` | | `~/.codex/config.toml` | |
| Cursor | `~/.cursor/skills` | `~/.cursor/agents` | `~/.cursor/mcp.json` | |
| OpenCode | `~/.config/opencode/skills` | `~/.config/opencode/agents` | `~/.config/opencode/opencode.json` | commands |
| IBM Bob | `~/.bob/skills` | | `~/.bob/mcp_settings.json` | rules, commands |

A tool is set up automatically when its folder exists (for example
`~/.cursor`). You can also turn tools on and off yourself:

```sh
lo targets                    # list them
lo targets enable pi
lo targets disable cursor
```

or on the web UI's **AI tools** page.

## How items get there

Loadout keeps one copy of every item it installs (its *store*) and
**links** it into each tool's folder: a symlink, or a junction on Windows.
Updates then show up in every tool at once. If a tool doesn't follow links,
switch it to copies:

```sh
lo targets mode claude-code copy
```

Tool connections (MCP servers) are merged into each tool's config file.
Loadout:

- only ever changes entries it created, never yours,
- writes the file safely (all or nothing) and keeps a backup next to it
  (`.loadout.bak`),
- never writes a password or token into it (see
  [Tool connections and secrets](Tool-Connections-and-Secrets.md)).

If you already have your own skill with the same name as one Loadout
would install, yours is left alone and `lo doctor` tells you about the
clash.

## Limiting an item to some tools

In an item's `loadout:` block:

```yaml
loadout:
  targets: [claude-code, cursor]
```

## Adding any other tool

If your AI tool reads skills from a folder (most do: they follow the Agent
Skills `SKILL.md` format), describe it in a small file in
`~/.config/loadout/targets/`:

```toml
# ~/.config/loadout/targets/my-agent.toml
id = "my-agent"
display = "My Agent"
detect = ["~/.my-agent"]

[skills]
path = "~/.my-agent/skills"
layout = "dir-per-item"

[mcp]
renderer = "cursor-json"   # the closest built-in format
path = "~/.my-agent/mcp.json"
key = "mcpServers"
```

[`docs/any-agent.md`](../any-agent.md) has every option, and
[`targets/`](../../targets) has the built-in files to copy from.

## Agent plugins

These are optional conveniences around the same `lo` command:

- **Claude Code:** `/plugin marketplace add Drewdl3/loadout`, then
  `/plugin install loadout@loadout`. Adds `/lo <command>`, a sync
  when a session starts, and skills that teach Claude how to use Loadout.
- **Pi:** copy `shims/pi/loadout` into `~/.pi/agent/extensions/`.
- **CI:** `uses: Drewdl3/loadout/shims/github-action@main` installs `lo`
  and runs `lo sync --locked`.

## Project mode

To give one code repo its own items (installed into that repo, for example
`.claude/skills`, rather than your home folder):

```sh
cd my-repo
lo init --project
lo --project subscribe https://git.example.com/acme/checkout-skills
lo --project sync
```

This creates `.loadout/` with the project's sources and a `loadout.lock` you
commit, so everyone working on the repo gets the same thing. Project items
rank as the `project` layer.

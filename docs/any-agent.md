# Using Loadout with any agent

`lo` is a standalone, static binary. It needs no daemon, no server and no
particular AI tool. The Claude Code plugin, the Pi extension and the GitHub
Action are thin conveniences around the same CLI. Anything that can run a
command can use it: a terminal, a script, CI, or an agent such as IBM Bob.

## 1. Tools Loadout installs into

Built-in targets (`lo targets` lists them):

- Claude Code
- Codex
- Cursor
- OpenCode
- Pi
- IBM Bob

A target is enabled when its directory exists (e.g. `~/.bob`) or you run
`lo targets enable <id>`. For IBM Bob:

| Item | Installed to |
|---|---|
| Skills | `~/.bob/skills/<name>/` |
| MCP servers | `~/.bob/mcp_settings.json` (`mcpServers`; your own entries are untouched) |
| `extras/rules/<name>.md` | `~/.bob/rules/<name>.md` |
| `extras/commands/<name>.md` | `~/.bob/commands/<name>.md` |

In project mode, the paths are `.bob/skills`, `.bob/mcp.json`, `.bob/rules`
and `.bob/commands` in the repo.

## 2. Any other tool: a target file

Drop a TOML file into `~/.config/loadout/targets/` (see
[`targets/`](../targets) for the built-ins; a file with a built-in's `id`
replaces it):

```toml
# ~/.config/loadout/targets/my-agent.toml
id = "my-agent"
display = "My Agent"
detect = ["~/.my-agent"]          # enabled automatically when this exists

[skills]                          # any tool that reads Agent Skills (SKILL.md dirs)
path = "~/.my-agent/skills"
layout = "dir-per-item"

[agents]                          # optional: one Markdown file per subagent
path = "~/.my-agent/agents"
layout = "file-per-item"

[extras.rules]                    # optional: extras/rules/*.md
path = "~/.my-agent/rules"
transform = "none"

[mcp]                             # optional: pick the closest native format
renderer = "cursor-json"          # claude-code-json | cursor-json | pi-mcp-json | opencode-json | codex-toml | bob-json
path = "~/.my-agent/mcp.json"
key = "mcpServers"
```

Then `lo targets enable my-agent && lo sync`. If the tool doesn't
follow symlinks, use `lo targets mode my-agent copy`.

## 3. Keeping up to date without a plugin

Use either of these:

- `lo schedule enable` installs an hourly OS scheduler entry (launchd,
  a systemd timer or cron, or Task Scheduler).
- A session-start hook in your agent runs
  `lo sync --if-stale --quiet --exit-zero`. For IBM Bob, add this to
  `~/.bob/settings/settings.json`:

  ```json
  {
    "hooks": {
      "SessionStart": [
        { "hooks": [{ "type": "command", "command": "lo sync --if-stale --quiet --exit-zero" }] }
      ]
    }
  }
  ```

## 4. Teaching the agent the CLI

The [`loadout` skill](../shims/claude-code/skills/loadout/SKILL.md) tells an agent
how to answer "where does this skill come from?", "turn X off", "what's
pending?" and so on with `lo … --json --exit-zero`. It's an ordinary
skill, so the simplest way to give it to every tool is to **distribute it
through your company config**. Copy it to `skills/loadout/SKILL.md` in the company config
repo, and every person gets it in every tool they use.

The [`loadout-authoring` skill](../shims/claude-code/skills/loadout-authoring/SKILL.md)
teaches an agent to write sources: item formats, templates, company configs, and
the checks to run before committing ([`docs/agents.md`](agents.md) is the long
version). Ship it the same way, with `loadout: { mode: default-off }` if only
the people who maintain sources should get it.

## 5. Tools without skills or MCP

Agents can read items straight from the CLI:

```sh
lo list --enabled --json                 # what this person has
lo info skill/write-spec                 # the skill's documentation, to paste or pipe into a prompt
lo search "pull request" --json          # find one
```

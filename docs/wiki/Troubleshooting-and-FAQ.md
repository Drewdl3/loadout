# Troubleshooting and FAQ

Start with:

```sh
lo doctor        # checks this computer and says what to do
lo status        # last sync, pending updates, conflicts
```

or the web UI's **Health check** page.

## "I don't have a skill I expected"

```sh
lo why skill/<name>
```

It lists every version and why each lost. The usual causes:

| What `why` shows | Meaning | Fix |
|---|---|---|
| no versions at all | None of your groups share it. | `lo search <name> --all-sources` to find which group does, then `lo join` it (if it's optional). |
| filtered out by `applies_to` | It's for a role or group you don't have. | Nothing to fix, it's not meant for you. Ask the owners if it should be. |
| the winner, but disabled | It's available but off by default. | `lo enable <id>`, or **Turn on** in the UI. |
| your expected version outranked or blocked | Another layer's version won. | See [How layers work](How-Layers-Work.md). |
| the winner, but not installed in a tool | It's limited to other tools (`targets`), or that tool isn't enabled. | `lo targets` |

Also check `lo status` for updates waiting for your review: a new skill
might be in one.

## "I'm not in the group I should be in"

```sh
lo profile
```

shows every group and how membership was decided (`rule`, `joined`, `left`,
`cached`). If it says `cached`, the check failed (offline, or no GitHub/GitLab
token) and the last known answer was used. Log in (`gh auth login`) and run
`lo profile refresh`. If the group is optional, `lo join <layer>:<group>`.

## "Two groups have the same skill" (exit code 4)

Two sources at the same rank both provide it, and neither has a higher
priority. Pick one:

```sh
lo why skill/write-spec
lo prefer payments-skills:skill/write-spec
```

or **Use this version** in the web UI.

## "My own skill was not replaced"

That's deliberate. Loadout never touches files it didn't create. If you have
`~/.claude/skills/write-spec` of your own, Loadout's `write-spec` isn't
installed for that tool and `lo doctor` reports the clash. Rename or remove
yours, or share it with `lo adopt`.

## "An update is stuck" (exit codes 2 and 3)

- `2`: changes are waiting for your review. `lo diff`, then
  `lo approve`.
- `3`: the safety audit blocked a change. Read `lo diff <source>` and,
  if you're sure it's fine, `lo approve <source> --force-audit`. Better:
  tell the source's owners.

## "The web UI says it can't reach the API"

The UI's address has a key that changes every time `lo ui` starts. Use the
address it just printed, and keep the terminal open while you use it.

## "Opening a pull request fails"

The source needs a GitHub remote and you need to be logged in:
`gh auth login`, or set `GH_TOKEN`.

## "Where does Loadout keep things?"

| Linux | What |
|---|---|
| `~/.config/loadout/config.toml` | Your settings, sources and on/off choices |
| `~/.local/share/loadout/` | Downloaded sources, the store, `loadout.lock`, state |

On macOS it's `~/Library/Application Support/loadout`, and on Windows
`%APPDATA%\loadout`. Set `LOADOUT_CONFIG_DIR`, `LOADOUT_DATA_DIR` and
`LOADOUT_HOME` to use other folders, for example to try things in a
sandbox.

## "How do I remove everything?"

Turn off each tool (`lo targets disable <id>`) and run `lo sync`:
Loadout removes the links and config entries it created. Then delete the
two folders above and the `lo` binary.

## FAQ

**Does Loadout run in the background?** No. It runs when you (or
`lo schedule`, or an agent plugin at session start) run it.

**Does it send my data anywhere?** No. It fetches from your Git sources and,
if your company config uses them, asks GitHub or GitLab which teams you're in.
`lo self-update` checks GitHub for new releases. That's all.

**Can I use it without a company config?** Yes. `lo subscribe` any source.

**Is it safe to run `lo sync` often?** Yes. It's idempotent: running it
twice in a row changes nothing the second time.

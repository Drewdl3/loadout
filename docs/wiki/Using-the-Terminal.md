# Using the terminal

Everything in Loadout is a `lo` command. The web UI and terminal UI run
these same commands underneath.

## The ones you'll use most

```sh
lo sync                        # fetch the latest and install it
lo list                        # what's installed
lo why skill/write-spec        # which version you got, and why
lo search spec                 # find items by name, description, tag or text
lo enable eng-skills:skill/incident-runbook
lo disable payments-skills:skill/pci-checklist
lo status                      # last sync, updates waiting, conflicts
lo doctor                      # check this computer for problems
```

## Groups

```sh
lo profile                     # the groups you're in and how each was decided
lo profile refresh             # ask GitHub/GitLab/your SSO again now
lo join product:billing        # opt into a group
lo leave product:billing
```

Joining and leaving stick, even when your groups are re-checked.

## Sources

```sh
lo subscribe <url> --dry-run   # preview what a source (and its upstreams) offers
lo subscribe <url>             # add it
lo unsubscribe <url>
lo info payments-skills        # a source's LOADOUT.md
lo info skill/write-spec       # an item's text
```

## AI tools

```sh
lo targets                     # which AI tools are set up
lo targets enable cursor
lo targets mode claude-code copy   # copy files instead of linking
```

## Updates

```sh
lo diff                        # what's waiting for your review
lo approve payments-skills     # or --all
lo schedule enable             # sync every hour using the OS scheduler
```

## Sharing

```sh
lo export                      # your exact setup as a short code (lo1_…)
lo import lo1_… --dry-run      # see what it would change first
lo import lo1_…                # reproduce someone's setup
lo adopt                       # list skills in your AI tools you could share
lo template list
lo export-source payments-skills -m "Improve write-spec"   # open a pull request
```

## The terminal UI

```sh
lo tui
```

A full-screen browser for the catalog (search and filters), the sources
tree, templates, your groups and pending updates. Toggle, prefer or join with
one key, with the `why` explanation alongside. Press `?` to see the keys.

## For scripts and AI agents

Every command accepts:

- `--json`: machine-readable output, with a schema per command in
  [`schemas/cli/`](../../schemas/cli)
- `--quiet`
- `--exit-zero`: always exit 0 (for hooks)
- `--project`: act on the current repo's project layer

Exit codes: `0` ok, `1` error, `2` changes are waiting for review, `3` a
safety audit blocked a change, `4` a conflict needs `lo prefer`.

[`docs/agents.md`](../agents.md) is written for AI agents that drive `lo`.
The [README](../../README.md#commands) lists every command and flag.

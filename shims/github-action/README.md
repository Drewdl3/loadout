# `setup-loadout` GitHub Action

Installs `lo` and, by default, runs `lo sync --locked --non-interactive`
with `LOADOUT_ROLE=ci` — the agent configuration pinned in `loadout.lock`,
with secrets from the job's environment (`env://` / `secret://` via
environment variables).

```yaml
- uses: Drewdl3/loadout/shims/github-action@main
  with:
    version: v0.1.0            # default: latest release
  env:
    LOADOUT_CONFIG_DIR: ${{ github.workspace }}/.loadout
    JIRA_TOKEN: ${{ secrets.JIRA_TOKEN }}
```

Inputs: `version`, `binary` (use an existing binary instead of a release),
`args` (default `sync --locked --non-interactive`; empty to only install),
`role` (default `ci`), `token`.

Source repos can audit themselves in CI with
`args: audit --path .` (what `lo new-source` sets up).

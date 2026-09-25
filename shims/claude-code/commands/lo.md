---
description: Run an Loadout command (why, list, status, diff, search, enable, disable, approve, doctor…) and explain the result.
argument-hint: "<command> [args]   e.g. why skill/write-spec"
allowed-tools: Bash(lo *)
disable-model-invocation: true
---

## `lo $ARGUMENTS`

!`lo $ARGUMENTS --json --exit-zero`

The JSON above is the output of `lo $ARGUMENTS` (Loadout distributes
skills, MCP servers and other agent configuration from the user's company
repos). Explain it briefly for the user:

- For `why`: which source won and why (layer rank, lock, priority), which
  candidates lost, and where it is installed.
- For `status` / `diff`: what is pending review and why; suggest
  `/lo approve <source>` only if the user wants it.
- For `list` / `search`: a short table of items (id, on/off, description).
- If the JSON has `"error"`, say what went wrong and how to fix it.

Keep it short. Don't run further commands unless the user asks.

# Loadout for Pi

`loadout/index.ts` is a Pi extension:

- `/lo <command> [args]` runs `lo <command> --json` and asks the model
  to explain the result (e.g. `/lo why skill/write-spec`).
- On session start it runs `lo sync --if-stale --quiet` in the background.

Install it by copying the directory into Pi's extensions directory:

```sh
mkdir -p ~/.pi/agent/extensions
cp -R shims/pi/loadout ~/.pi/agent/extensions/
```

`lo` must be on `PATH`. MCP servers reach Pi through the
[`pi-mcp-adapter`](https://github.com/nicobailon/pi-mcp-adapter) extension.

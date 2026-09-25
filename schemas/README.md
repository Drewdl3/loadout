# JSON Schemas

Published JSON Schemas (draft 2020-12) for Loadout's formats:

| File | Describes | Status |
|---|---|---|
| `manifest.schema.json` | `LOADOUT.md` frontmatter (source manifest + company config block) | generated |
| `item-block.schema.json` | The `loadout:` block in item frontmatter | generated |
| `template.schema.json` | A template's `SKILL.md` frontmatter (`templates/<name>/`, with its `template:` block) | generated |
| `config.schema.json` | `~/.config/loadout/config.toml` (TOML; the schema describes its data model) | generated |
| `lock.schema.json` | `loadout.lock` (TOML; the schema describes its data model) | generated |
| `cli/<command>.schema.json` | `--json` output of each CLI command (`error.schema.json` on failure) | generated |

All schemas are generated from the Rust types; a test fails if one is stale
(regenerate with `UPDATE_SCHEMAS=1 cargo test -p loadout-cli --test schemas`).

Schemas are versioned alongside the `loadout: 1` manifest schema version.

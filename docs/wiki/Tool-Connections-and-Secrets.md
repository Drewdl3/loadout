# Tool connections and secrets

A **tool connection** (an MCP server) lets your AI assistant use another
system: Jira, a database, your docs. Loadout shares these like skills,
with one important rule: **passwords and tokens never go in the shared
files, and never get written to disk.**

## What a shared connection looks like

`mcp/jira.md` in a source:

```markdown
---
name: jira
description: Acme Jira (tickets, sprints).
command: npx
args: ["-y", "@acme/jira-mcp"]
env:
  JIRA_BASE_URL: "https://acme.example.com"
  JIRA_TOKEN: "secret://jira/token"
---
```

`secret://jira/token` is a *reference*, not the token. Each person's own token
stays on their own computer.

HTTP servers work the same way, with `url:` and `headers:` instead of
`command:`.

## Giving Loadout your token

```sh
lo secrets set jira/token      # asks for it; saved in your OS keychain
lo secrets check               # does every reference have a value?
```

Or set an environment variable: `secret://jira/token` also looks for
`JIRA_TOKEN`.

## How the secret reaches the tool

When a connection needs a secret, Loadout writes it into your AI tool's
config so that the tool starts it through `lo mcp-run <id>`. When the tool starts the server,
`lo mcp-run` looks up your token in memory and then runs the real command
with it. For HTTP servers, the tool asks `lo mcp-headers <id>` for the
headers. Either way, the token exists only in memory.

Writing a real token into a tool's config file is a last resort. It needs
`allow_secrets_on_disk = true` in your `config.toml`, and `lo doctor` flags
it.

## Where secrets can come from

| Reference | Looks in |
|---|---|
| `secret://name` | A chain: an environment variable (`name` in capitals, other characters as `_`, so `jira/token` → `JIRA_TOKEN`), then your OS keychain. The company config or your config can add more, such as 1Password or Vault. |
| `env://VAR` | Environment variable `VAR` |
| `keychain://service/account` | The OS keychain (macOS Keychain, Windows Credential Manager, Secret Service on Linux) |
| `op://vault/item/field` | 1Password CLI |
| `vault://path#field` | HashiCorp Vault |
| `sops://file.enc.yaml#key` | A SOPS-encrypted file |

A company config sets the chain for everyone:

```yaml
company:
  secrets:
    chain: [env, keychain]
```

## Checking nothing leaked

- `lo secrets check` lists every reference and whether it resolves (never
  the value).
- `lo doctor` warns about any secret written to disk.
- `lo audit` flags tokens committed to a source.
- Shortcodes from `lo export` never contain secrets.

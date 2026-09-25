---
name: jira
description: Acme Jira (tickets, sprints). The token is a secret reference, never a value.
loadout:
  mode: required
  tags: [jira, tickets]
transport: stdio
command: npx
args: ["-y", "@acme/jira-mcp"]
env:
  JIRA_BASE_URL: "https://jira.example.com"
  JIRA_TOKEN: "secret://jira/token"
---

Connects the agent to Acme's Jira. Each person provides their own token:
`lo secrets set jira/token` (stored in the OS keychain) or the
`JIRA_TOKEN` environment variable (CI). `lo mcp-run` resolves it when
the agent starts the server; it is never written to disk.

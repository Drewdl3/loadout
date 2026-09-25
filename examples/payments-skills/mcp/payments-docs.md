---
name: payments-docs
description: Search Acme's internal payments documentation.
url: https://mcp.example.com/payments-docs
headers:
  Authorization: "Bearer env://PAYMENTS_DOCS_TOKEN"
---

An HTTP MCP server. The bearer token comes from the `PAYMENTS_DOCS_TOKEN`
environment variable; in Claude Code Loadout supplies it at connection
time through `lo mcp-headers`.

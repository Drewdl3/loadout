# Audit rules

Content-audit rules, compiled into `lo`. `lo audit` runs
them over your sources; `lo sync` runs them over changed items before
applying.

- `default.toml` — built-in rules: prompt injection, hidden Unicode,
  obfuscation, exfiltration, dangerous commands (`curl | sh`, `rm -rf /`),
  unpinned `npx -y`, and committed secrets. Adapted from
  [runkids/skillshare](https://github.com/runkids/skillshare) (MIT); see
  `THIRD_PARTY_NOTICES.md`.
- A built-in `secret-high-entropy` check flags random-looking tokens
  (32–100 characters, mixed case and digits, not hex).

Severities: `info`, `warn`, `high` (blocks auto-apply: the change waits for
`lo approve`), `critical` (blocks applying unless `--force-audit`).

## Company rules

A company config can add rules with `audit_rules: <path>` (a `.toml` file or a
directory of them, inside the company config repo). Same format:

```toml
[[rule]]
id = "acme-prod-db"
severity = "high"            # info | warn | high | critical
category = "acme"            # optional
message = "Mentions the production database"
regex = 'db-prod\.acme\.internal'   # Rust regex, matched per line
exclude = 'example'          # optional: matching lines are ignored
kinds = ["skill", "agent"]   # optional: skill, mcp, agent, plugin, extra
secret = false               # optional: redact the match in reports

# Switch a built-in rule off by repeating its id:
[[rule]]
id = "untrusted-install-0"
enabled = false
```

Static audit is a speed bump, not a guarantee: review policy and pinning
are the real controls.

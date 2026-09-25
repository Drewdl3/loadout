# AGENTS.md — Loadout

Guidance for AI agents (and people) working on **Loadout** (`lo`), a Rust CLI
that distributes AI-agent configuration (skills, MCP servers, subagents,
plugins, rules) across a company from federated Git repos.

- **Behavior:** `README.md` and `docs/` describe it: the user guide in
  `docs/wiki/`, the formats in `docs/agents.md`, the JSON Schemas in
  `schemas/`. The tests pin it down.
- **Work items:** GitHub issues. New ideas and follow-ups become issues, not
  notes in the repo.

## Layout

| Path | What |
|---|---|
| `crates/loadout-model` | File formats: `LOADOUT.md`, item frontmatter, the company config, `config.toml`, `loadout.lock`. |
| `crates/loadout-core` | Resolution (which version of each item wins) and other pure logic. |
| `crates/loadout-git` | Fetching and pinning sources. |
| `crates/loadout-members` | Group membership discovery (GitHub, GitLab, env, exec, repo access). |
| `crates/loadout-targets` | Installing into each AI tool (links, MCP configs, plugins). |
| `crates/loadout-secrets` | `secret://` references and resolver chains. |
| `crates/loadout-audit`, `loadout-search`, `loadout-code` | Content audit, search, shortcodes. |
| `crates/loadout-cli` | The `lo` binary, the terminal UI and the local web UI. |
| `targets/`, `audit/`, `schemas/`, `shims/`, `examples/`, `docs/` | Target definitions, audit rules, JSON Schemas, agent integrations, a sample company, docs. |

## Checks

Run these before every push; all must pass:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- CLI JSON schemas are generated: `UPDATE_SCHEMAS=1 cargo test -p loadout-cli --test schemas`.
- Snapshots use `insta`: review every changed `.snap` before accepting it.

## Workflow

1. Branch from `main`: `feat/<name>`, `fix/<name>` or `chore/<name>`.
2. Make the change with tests, small enough to review in one sitting.
3. Run the checks, push, and open a PR that links the issue.
4. Squash-merge once CI is green.

Never disable a failing check to get green, and never mark a test
`#[ignore]` without an issue to fix it.

## Rules

**Correctness and tests**
- `loadout-core` is pure: no filesystem, network, clock or environment
  access. Inject everything. It carries the most tests, including the
  property test that resolution doesn't depend on source order.
- Tests never touch the real home directory or real agent configs. Use temp
  dirs and a fake home (`LOADOUT_HOME`, `LOADOUT_CONFIG_DIR`,
  `LOADOUT_DATA_DIR`).
- Tests never use the network. Build fixture Git repos locally in temp dirs.
- Use `insta` snapshots for CLI output, `why` explanations and rendered
  target configs.

**Third-party tools**
- Before relying on how a tool (Claude Code, Pi, Codex, Cursor, OpenCode,
  Bob) lays out its files or config, check its current docs, and link them
  in the PR.
- If you copy or port code or data from another project, add its copyright
  and license to `THIRD_PARTY_NOTICES.md` and list the derived files.

**Safety**
- Never write to your real `~/.claude`, `~/.pi`, `~/.codex`, … while
  developing. Manual end-to-end checks run in a sandbox `HOME`.
- Never commit secrets, tokens or real company URLs. Use `example.com` /
  `acme` fixtures.
- Writes to user config files are atomic and backed up (`.loadout.bak`).

**Compatibility**
- The public formats (`LOADOUT.md`, the company config, item frontmatter,
  `config.toml`, `loadout.lock`, shortcodes, CLI and `--json` output) change
  only additively. A breaking change needs the maintainers' agreement first.
- Explain non-obvious choices in the PR description and, where a reader of
  the code needs it, in a short comment.

## Style

- Errors: `thiserror` in library crates, `anyhow` only in `loadout-cli`.
- Logging via `tracing`; never log secret values (use `secrecy`).
- Every public CLI command supports `--json`, with a schema in `schemas/`.
- Commit messages: conventional commits (`feat(core): …`, `fix(targets): …`).
- Keep `README.md` and `docs/wiki/` current with the commands and UI.

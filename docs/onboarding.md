# Onboarding: a company, a new team, a new developer

This walks through Loadout from each person's side. Every step here runs
for real in [`examples/onboard.sh`](../examples/onboard.sh), which uses a
throwaway sandbox and local repos, and in the integration test
`crates/loadout-cli/tests/onboarding.rs`.

```sh
cargo build && LOADOUT=$PWD/target/debug/lo ./examples/onboard.sh
```

## 0. The platform team: one company config for the company (once)

```sh
lo init --new-company acme-config --company acme --git-init
```

This scaffolds a company config: the layers and their ranks (company → org → team →
product → squad → project → role → user, or your own with
`--layers company=0,division=10,team=20,user=50`), an empty `groups:` list with an
example, and a policy (company updates apply automatically; everything else
waits for review). The platform team pushes it and puts company-wide items
in it. Good first items:

- **The `loadout` skill** ([`examples/acme-config/skills/loadout`](../examples/acme-config/skills/loadout/SKILL.md)).
  Every AI tool of every employee then knows how to answer "where does this
  skill come from?" or "turn X off" with the CLI.
- Required, locked standards (`loadout: { mode: required, locked: true }`).
- Shared MCP servers with secret *references* (`secret://jira/token`).

Orgs and teams are added to `company.groups` as they onboard. Each group lists
its repo and a membership rule (`github_team`, `gitlab_group`, `exec` for
Okta/Entra/LDAP, `env`, `manual`).

## 1. A new squad sets up its catalog

Sam leads the new Checkout squad inside the Payments team. Sam already uses
Loadout (`lo init <company-config-url>`).

**Find what the higher layers offer:**

```sh
lo template list            # eng-skills:template/pr-workflow — variables: team, reviewers, branch_prefix
lo search "pull request" --all-sources
```

**Create the squad's source, building on the Payments team's:**

```sh
lo init --new-source checkout-squad --layer squad --group checkout \
  --upstream https://git.example.com/acme/payments-skills --git-init
```

`upstream:` in the new `LOADOUT.md` means that anyone who subscribes to the
squad's source directly also gets the Payments (and, through it,
Engineering) layers. Company config members get those anyway.

**Adapt Engineering's PR template into the squad's own skill:**

```sh
lo template use pr-workflow --dir checkout-squad --name checkout-prs \
  --set team=Checkout --set reviewers=1
lo audit --path checkout-squad
```

The result is an ordinary skill owned by the squad, recording where it came
from (`loadout.from_template`). Commit and push it. If the source is already
subscribed, `--into checkout-squad` followed by `lo export-source
checkout-squad` opens the pull request for you. The same flow is available
in `lo ui` (Templates) and `lo tui` (Templates tab).

**Register the squad in the company config** with a pull request to the company config
repo:

```yaml
    - layer: squad
      name: checkout
      sources: ["https://git.example.com/acme/checkout-squad"]
      membership: { github_team: "acme/checkout" }   # the example uses { manual: true }
```

## 2. A new developer joins

Alex's first day:

```sh
curl -fsSL https://raw.githubusercontent.com/Drewdl3/loadout/main/install.sh | sh -s -- \
  --company-config https://git.example.com/acme/agent-config
```

That runs `lo init`. It discovers Alex's org, team and role from the
company config's membership rules, lets Alex adjust groups and pick AI tools, and
installs everything: company standards, the Engineering and Payments
skills, MCP servers, subagents and commands. If the squad's rule is opt-in:

```sh
lo join squad:checkout      # installs the squad's items now
lo why skill/checkout-prs   # where it comes from
lo tui                      # browse, filter by role/tag/source, toggle
```

**A personal layer.** Alex wants a different spec format:

```sh
lo init --new-source ~/src/alex-skills --layer user \
  --upstream https://git.example.com/acme/checkout-squad --subscribe
# add skills/write-spec/SKILL.md, commit
lo sync --yes
lo why skill/write-spec     # alex-skills wins: user outranks team and org
```

A company item marked `locked` would still win. Personal layers can't
override policy.

## 3. Staying up to date

```sh
lo schedule enable          # hourly, via launchd / systemd / Task Scheduler
```

When Sam changes the squad's skill, the change doesn't land silently. It is
held for review (only `company` auto-applies in this company config):

```sh
lo status                   # 1 change pending: checkout-squad …
lo diff checkout-squad      # the Git diff and audit findings
lo approve checkout-squad
```

## 4. Other tools

The same setup installs into Claude Code, Codex, Cursor, OpenCode, Pi and
IBM Bob, and into any other tool through a small target file. See
[`any-agent.md`](any-agent.md).

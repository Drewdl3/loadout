# Onboarding: a squad, a new developer, and a company config later

This walks through Loadout from each person's side, the way it usually
spreads: team by team, and only later (if ever) with a company-wide setup.
Every step here runs for real in [`examples/onboard.sh`](../examples/onboard.sh),
which uses a throwaway sandbox and local repos, and in the integration test
`crates/loadout-cli/tests/onboarding.rs`.

```sh
cargo build && LOADOUT=$PWD/target/debug/lo ./examples/onboard.sh
```

## 0. Where Acme starts

Acme's Engineering org and its Payments team already keep their skills in
source repos ([`examples/`](../examples/README.md)). Payments builds on
Engineering's:

```yaml
# payments-skills/LOADOUT.md
name: payments-skills
layer: team
group: payments-dev
upstream: [../eng-skills]
```

There's no company config yet, and nobody needs one.

## 1. A squad lead connects, and sets up the squad's catalog

Sam leads the new Checkout squad inside the Payments team.

**Connect to the team's source** with the link a teammate shared:

```sh
lo init https://git.example.com/acme/payments-skills
```

Sam sees what the source brings, including `eng-skills` (its upstream),
confirms, picks AI tools, and everything is installed.

**Find what the higher layers offer:**

```sh
lo template list            # eng-skills:template/pr-workflow — variables: team, reviewers, branch_prefix
lo search "pull request"
```

**Create the squad's source, building on the Payments team's:**

```sh
lo init --new-source checkout-squad --layer squad --group checkout \
  --upstream https://git.example.com/acme/payments-skills --git-init
```

`upstream:` in the new `LOADOUT.md` means anyone who connects to the squad's
source also gets the Payments layer and, through it, Engineering's.

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

## 2. A new developer joins

Alex's first day. Sam sends one link, the squad's:

```sh
curl -fsSL https://raw.githubusercontent.com/Drewdl3/loadout/main/install.sh | sh -s -- \
  --connect https://git.example.com/acme/checkout-squad
```

That runs `lo init <link>`: the squad's source, Payments' and Engineering's,
with their skills, MCP servers, subagents and commands, in every AI tool
Alex uses. (Or: `lo ui`, **Paste a link**.)

```sh
lo why skill/checkout-prs   # where it comes from
lo tui                      # browse, filter by role/tag/source, toggle
```

**A personal layer.** Alex wants a different spec format:

```sh
lo init --new-source ~/src/alex-skills --layer user \
  --upstream https://git.example.com/acme/checkout-squad --subscribe
# add skills/write-spec/SKILL.md, commit
lo sync --yes
lo why skill/write-spec     # alex-skills wins: user outranks squad, team and org
```

An item marked `locked` would still win. Personal layers can't override
policy, and neither can re-ranking.

**Their own order.** Alex also works with the Beta team and wants its
skills to win over the squad's (Beta isn't part of the example):

```sh
lo subscribe https://git.example.com/acme/beta-skills --label "Beta" --rank 35
lo layers                   # every rank, and where it came from
```

## 3. Staying up to date

```sh
lo schedule enable          # hourly, via launchd / systemd / Task Scheduler
```

When Sam changes the squad's skill, the change doesn't land silently. It is
held for review (without a company config, every update is):

```sh
lo status                   # 1 change pending: checkout-squad …
lo diff checkout-squad      # the Git diff and audit findings
lo approve checkout-squad
```

To skip review for sources you trust, list them under `[policy] auto_apply`
in your `config.toml`.

## 4. Later: a company config

A few teams in, Acme's platform team wants people's groups worked out from
their GitHub teams and company-wide rules (security standards everyone gets,
signed commits for the company layer). They add a company config listing the
team sources that already exist:

```sh
lo init --new-company acme-config --company acme --git-init
```

```yaml
    - layer: squad
      name: checkout
      sources: ["https://git.example.com/acme/checkout-squad"]
      membership: { github_team: "acme/checkout" }   # the example uses { manual: true }
```

People connect to it on top of what they have:

```sh
lo init https://git.example.com/acme/agent-config
```

Alex keeps the squad, Beta and personal sources; the company config adds
the company's items (a locked review standard, the `loadout` skill that
teaches AI tools the CLI) and works out Alex's org, team and role. New
people can still start from their team's link. See
[Growing across your org](wiki/Growing-Across-Your-Org.md).

## 5. Other tools

The same setup installs into Claude Code, Codex, Cursor, OpenCode, Pi and
IBM Bob, and into any other tool through a small target file. See
[`any-agent.md`](any-agent.md).

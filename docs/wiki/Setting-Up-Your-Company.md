# Setting up your company

This page is for whoever looks after AI tools at your company. With a
**company config**, every developer gets set up with one link:
`lo init <company config-link>`.

## What a company config is

A company config is a source repo whose `LOADOUT.md` has a `company:` block listing:

- your **layers** (layers) and their ranks,
- every **group** (group), the source repos it publishes, and **who belongs**
  to it,
- **policy**: which updates apply automatically, whether people may add their
  own sources, which commits must be signed.

It can also hold company-wide items (skills everyone gets) like any other
source.

## Create one

```sh
lo init --new-company acme-config --company acme --git-init
```

Choose your own layer names and ranks with `--layers` (they must include
`company`):

```sh
lo init --new-company acme-config --company acme \
  --layers company=0,division=10,chapter=15,team=20,user=50
```

## An example

```yaml
---
loadout: 1
name: acme-config
layer: company
group: acme
owners: ["@acme/platform"]
company:
  name: acme
  layers:
    - { name: company, rank: 0 }
    - { name: org,     rank: 10 }
    - { name: team,    rank: 20 }
    - { name: role,    rank: 40 }
  groups:
    - layer: org
      name: eng
      sources: ["https://git.example.com/acme/eng-skills"]
      membership: { github_team: "acme/engineering" }
    - layer: team
      name: payments-dev
      sources: ["https://git.example.com/acme/payments-skills"]
      membership: { any: [ { github_team: "acme/payments" }, { manual: true } ] }
    - layer: role
      name: product-manager
      sources: ["https://git.example.com/acme/pm-presets"]
      membership: { exec: "acme-groups" }
  policy:
    auto_apply: [company]
    allow_manual_sources: true
  secrets:
    chain: [env, keychain]
---
```

The complete, runnable version is in
[`examples/acme-config`](../../examples/acme-config/LOADOUT.md).

## Deciding who's in each group

| Rule | Someone is a member when |
|---|---|
| `{ github_team: "org/team" }` | GitHub says they're an active member of the team. |
| `{ gitlab_group: "group/sub" }` | GitLab says they're a member of the group. |
| `{ exec: "command" }` | The command prints a JSON list of group names that includes this one. This is how you connect Okta, Entra ID or LDAP; see [membership recipes](../membership-recipes.md). |
| `{ env: { VAR: value } }` | An environment variable matches (handy for CI). |
| `{ repo_access: true }` | They can read the group's first source repo. |
| `{ manual: true }` | They choose to join it (`lo join`, or **Join** in the web UI). |
| `{ any: [...] }` / `{ all: [...] }` | Combine rules. |

Tokens for GitHub and GitLab come from the person's own login (`GH_TOKEN`,
`gh auth token`, `GITLAB_TOKEN`, or their Git credential helper) and are
never stored. If a check fails (offline, no token), the last known answer is
kept. Groups are re-checked at most once a day by default
(`profile_refresh_interval`).

People can always see how their groups were decided with `lo profile`, or
under **Your groups** on the web UI's Home page.

## Policy

- `auto_apply: [company]`: updates to these layers install straight away.
  Updates from every other layer wait for each person to approve them (see
  [Updates and review](Updates-and-Review.md)).
- `allow_manual_sources: true`: people may add sources that aren't in the
  company config.
- `require_signed: [company]` with `signers:`: only accept commits signed by
  these SSH or GPG keys for these layers. Anything else is refused and the
  previous version is kept.

## Rolling it out

1. Push the company config and each group's source repo to your Git host.
2. Put company-wide essentials (for example the `loadout` skill, which teaches
   AI tools how to use Loadout) in the company config itself.
3. Send people the install command with the company config link:
   ```sh
   curl -fsSL https://raw.githubusercontent.com/Drewdl3/loadout/main/install.sh | sh -s -- --company-config https://git.example.com/acme/agent-config
   ```
   or tell them to run `lo ui` and paste the link on **Start here**.
4. Suggest `lo schedule enable` so everyone stays current.

[`docs/onboarding.md`](../onboarding.md) walks through the whole thing end to
end, including a new squad and a new developer.

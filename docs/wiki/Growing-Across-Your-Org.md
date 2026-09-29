# Growing across your org

Loadout usually spreads team by team. One team starts a source, people
connect to it with `lo init <link>`, and then it connects to the teams
around it. You don't need anyone's permission, or a central setup, to start.

This page covers how team sources connect to each other, how layers get
their names and ranks from the bottom up, and when (if ever) a *company
config* is worth adding.

## 1. Start with one team

A team keeps its skills in a source repo: a Git repo with a `LOADOUT.md` at
its top.

```sh
lo init --new-source payments-skills --layer team --group payments-dev --subscribe
```

Push it to your Git host and share the link. Teammates run
`lo init <link>` (or paste the link on **Start here** in `lo ui`): they're
subscribed, their AI tools are set up, and they're in the team's group.
See [Sharing skills](Sharing-Skills.md) for writing the skills themselves.

## 2. Connect to the teams around you

When another team has skills yours should get (a platform team's runbooks,
your org's coding standards), list its source under `upstream:`:

```yaml
---
loadout: 1
name: payments-skills
layer: team
group: payments-dev
upstream:
  - https://git.example.com/acme/eng-skills        # our org's source
  - ../platform-skills                             # a sibling repo on the same host
---
```

Everyone subscribed to `payments-skills` then gets `eng-skills` and
`platform-skills` too, each at its own layer, and whatever *they* build on.
Nobody else needs to change anything.

An upstream entry can also say where that source sits for your subscribers:

```yaml
upstream:
  - url: https://git.example.com/acme/beta-skills
    label: Beta (payments)   # shown instead of its name
    layer: product           # rank all its items at the product layer
    rank: 27                 # …or at exactly this rank
    priority: 10             # breaks ties at the same rank
```

Each subscriber can still change that for themselves (see step 4).

## 3. Name your own layers

Layers rank sources: when two provide something with the same name, the
higher rank (closer to you) wins. Loadout has built-in layers
(`company` 0, `org` 10, `team` 20, `product` 25, `squad` 30, `project` 35,
`role` 40, `user` 50), and a source can add its own:

```yaml
---
loadout: 1
name: beta-skills
layer: beta
layers:
  - { name: beta, rank: 27 }   # our suggested rank
---
```

The ranks in effect merge, later steps winning:

1. the built-in defaults,
2. the `layers:` of the sources you're subscribed to (if two disagree, the
   higher rank is used and `lo sync` warns; a source can't rank `company`,
   or anything at or below it, so its locks can never beat the company's),
3. the company config's `company.layers`, if there is one,
4. your own `[layers]` in `config.toml` (`lo layers set beta=27`).

`lo layers` lists them, each with its rank and where that rank came from.
`user` stays above everything else unless something ranks it.

## 4. Everyone decides where things sit for them

Two people on the same team can rank things differently. Someone on two
products might want one to win over the other:

```sh
lo layers set beta=27                          # re-rank a whole layer
lo source set beta-skills --rank 35            # re-rank one source
lo source set beta-skills --label "Beta (payments)"
lo source set payments-skills --priority 10    # win ties at the same rank
lo source unset beta-skills --rank             # back to its own rank
```

This works for any source you get: your own subscriptions, upstreams, and
sources a company config lists. It's all in your `config.toml`
(`[layers]` and `[[source]]`), and `lo why <item>` names the setting that
decided a rank. In the web UI: **Place…** on the **Sources** page, and
**Ranks** on **Your layers**.

### Locks hold where they were declared

An item marked `locked: true` (or `overridable: false`) can't be replaced by
more specific layers. Re-ranking never gets around that: locks are compared
at the rank their source *declared* (in its `LOADOUT.md` or the company
config), not the one you gave it. Moving a locked company source below your
team, or a random source above the company, changes nothing about which
lock wins. `lo why` says when a re-rank was ignored because of a lock.

## 5. When is a company config worth adding?

Maybe never. Sources connected through `upstream:` cover most setups. A
**company config** is a source whose `LOADOUT.md` also has a `company:`
block. Add one when you want:

- **groups worked out automatically**: people get their team's sources
  from their GitHub team, GitLab group, Okta or LDAP groups, instead of
  subscribing by hand;
- **company-wide rules**: which layers' updates install without review,
  which commits must be signed, whether people may add other sources or
  re-rank the ones you list;
- **one link for everyone**, however many teams there are.

People who already use Loadout keep their subscriptions: `lo init <company
config link>` adds the company config on top. Anyone can still connect to
a single team's source by its link.

### Create one

```sh
lo init --new-company acme-config --company acme --git-init
```

Choose your own layer names and ranks with `--layers` (they must include
`company`). They override the built-in ranks and the sources' suggestions:

```sh
lo init --new-company acme-config --company acme \
  --layers company=0,division=10,chapter=15,team=20,user=50
```

### An example

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
    allow_local_ranks: true
  secrets:
    chain: [env, keychain]
---
```

The complete, runnable version is in
[`examples/acme-config`](../../examples/acme-config/LOADOUT.md).

If a team keeps its squads in its own repo (with `nested:`, see
[Sharing skills](Sharing-Skills.md#one-repo-for-a-team-and-its-squads)),
list the repo under the team's group only. Each squad's group needs just a
membership rule and no `sources`, because its items come with the team's
repo:

```yaml
    - layer: squad
      name: beaver
      membership: { github_team: "acme/beaver" }
```

The repo updates as one, so its changes skip review only if every layer in
it (here `team` and `squad`) is in `auto_apply`.

### Deciding who's in each group

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

### Policy

- `auto_apply: [company]`: updates to these layers install straight away.
  Updates from every other layer wait for each person to approve them (see
  [Updates and review](Updates-and-Review.md)).
- `allow_manual_sources: true`: people may add sources that aren't in the
  company config.
- `allow_local_ranks: true`: people may re-rank the company config's layers
  and the sources it lists for themselves. With `false`, those keep the
  company config's ranks (people can still label them, and rank layers and
  sources of their own). Locks hold at their declared rank either way.
- `require_signed: [company]` with `signers:`: only accept commits signed by
  these SSH or GPG keys for these layers. Anything else is refused and the
  previous version is kept.

### Rolling it out

1. Push the company config to your Git host, and list the team sources that
   already exist under their groups.
2. Put company-wide essentials (for example the `loadout` skill, which teaches
   AI tools how to use Loadout) in the company config itself.
3. Send people the install command with the link:
   ```sh
   curl -fsSL https://raw.githubusercontent.com/Drewdl3/loadout/main/install.sh | sh -s -- --connect https://git.example.com/acme/agent-config
   ```
   or tell them to run `lo ui` and paste the link on **Start here**.
4. Suggest `lo schedule enable` so everyone stays current.

[`docs/onboarding.md`](../onboarding.md) walks through the whole thing end to
end: a team starting on its own, a second team connecting, and a company
config added later.

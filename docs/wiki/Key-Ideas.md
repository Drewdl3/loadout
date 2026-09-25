# Key ideas

Five ideas explain almost everything Loadout does.

## Items: what gets shared

An **item** is one thing an AI assistant can use:

| Kind | In the web UI | What it is |
|---|---|---|
| `skill` | Skill | Written instructions for a particular job: "write a spec our way", "what to do when paged". A folder with a `SKILL.md`. |
| `mcp` | Tool connection | Lets the assistant use another tool, such as Jira or a database (an MCP server). |
| `agent` | Subagent | A helper assistant with its own instructions. |
| `plugin` | Plugin | A Claude Code plugin. |
| `extra` | Rule or prompt | House rules, saved prompts, slash commands. |

Every item has a name, like `skill/write-spec`. Its full id adds the source it
comes from: `payments-skills:skill/write-spec`.

## Sources: where items live

A **source** is a Git repo of items, usually one per group. It has a
`LOADOUT.md` file at the top saying who it's for, and items in folders:

```text
LOADOUT.md                 # name, layer and group (who it's for)
skills/<name>/SKILL.md
mcp/<name>.md
agents/<name>.md
plugins/<name>/
extras/<type>/<name>.md
templates/<name>/        # fill-in-the-blanks skills (never installed directly)
```

Because sources are ordinary Git repos, you get history, reviews (pull
requests) and ownership for free.

A source can **build on** another one with `upstream:`. The Checkout squad's
source can name Payments' source as its upstream, and anyone who adds the
Checkout source gets Payments' items too.

## Groups and layers: who gets what

A **group** (the CLI and file formats call it a *group*) is a set of people: the company,
Engineering, the Payments team, product managers, or just you.

A **layer** (a *layer*) is a kind of group, with a **rank**:

| Layer | Rank (default) | Example |
|---|---|---|
| `company` | 0 | acme |
| `org` | 10 | eng |
| `team` | 20 | payments-dev |
| `product` | 25 | billing |
| `squad` | 30 | checkout |
| `project` | 35 | one code repo |
| `role` | 40 | developer, product-manager |
| `user` | 50 | you |

A higher rank is more specific, closer to you. A company can choose its own
layer names and ranks.

You get an item when you're in its group. When two layers have an item with
the same name, the higher rank wins, unless the lower one **locked** it. That's
the whole of [How layers work](How-Layers-Work.md).

## The company config: the company's list of groups

A **company config** is one special source that lists every group, its sources, and
how to tell who belongs to it (GitHub or GitLab teams, your SSO, an
environment variable, or "join it yourself"). With a company config, one link sets
everyone up: `lo init <company config-link>`. Without one, people add sources
themselves with `lo subscribe`.

See [Setting up your company](Setting-Up-Your-Company.md).

## AI tools: where items end up

An **AI tool** (a *target*) is an app on your computer that Loadout installs
into: Claude Code, Cursor, Codex, Pi, OpenCode, IBM Bob, or any tool you
describe in a small file. Loadout keeps one copy of each item and links it
into each tool, in that tool's format. It only ever touches things it created;
your own skills are left alone.

See [AI tools](AI-Tools.md).

## Putting it together

```mermaid
flowchart LR
  company config["Company config<br/><i>lists groups and sources</i>"] --> groups["Your groups<br/><i>company, org, team, role</i>"]
  groups --> sources["Their sources<br/><i>Git repos of items</i>"]
  sources --> layers["Stacked as layers<br/><i>closest to you wins</i>"]
  layers --> tools["Your AI tools"]
```

# Sharing skills

Wrote a prompt that works well? Turn it into a skill your whole team gets
automatically. Nothing reaches anyone until the source's owners accept it in a
pull request.

## The quickest way: the web UI

1. `lo ui`, then **Create & edit**.
2. **New skill**: pick the source (your team's), give it a name and a short
   description of when to use it, and write the instructions.
3. **Open a pull request**. The owners review it; once merged, your team gets
   it on their next update.

Until then the skill lives only in your working copy of the source (a clone
under Loadout's data folder, not any checkout of the repo you may have). It
won't show in Browse skills yet; find it under **Create & edit → Not shared
yet**, where you can keep editing it and open the pull request later.

Already have skills in `~/.claude/skills` or another tool's folder? Use
**Share skills you already have** (or `lo adopt`) to copy them into a
source instead of retyping them.

## What a skill looks like

A skill is a folder with a `SKILL.md` in the source's `skills/` folder:

```markdown
---
name: write-spec
description: Write a feature spec in the Payments team's format. Use when asked for a spec, design doc or RFC.
loadout:
  tags: [specs, writing]
  author: Ada Lovelace
  co_authors: [Grace Hopper]
---

# Writing a spec

1. Start with the problem, in one paragraph, and who has it.
2. …
```

- `description` matters most: AI tools read it to decide when to use the
  skill. Say what it does *and when to use it*.
- The `loadout:` block is optional. Use it to set the mode (`default-off`,
  `required`), lock it, tag it, or narrow who gets it. See
  [How layers work](How-Layers-Work.md#setting-an-items-layer-mode-and-audience).
- `author` and `co_authors` say who wrote it. They're shown in the web UI and
  `lo info`, and `lo search --author <name>` finds everything someone
  wrote or co-wrote. A source can set a default author for all its items in
  `LOADOUT.md` under `defaults:`.
- You can add other files next to `SKILL.md` (checklists, examples, scripts)
  and refer to them from the text.

## Promoting a skill, or making your own version

Skills move between layers in both directions. In the web UI, click **Copy to
another layer…** on any item:

- **Promote** a skill that works well for your team to a broader source (your
  org's, the company's) so more people get it. It's a pull request to that
  source, so its owners decide.
- **Clone down** a company or org skill into your team's source, or your own
  personal one, and change it there. Your version replaces the original for
  your group (unless the original is locked), and you're added as a
  co-author.

Both are ordinary edits in working copies: open the pull requests from the
result panel when you're ready.

## Making a new source

Every group needs its own source. To create one for your team:

```sh
lo init --new-source payments-skills --layer team --group payments-dev \
  --upstream https://git.example.com/acme/eng-skills --git-init
```

This creates a folder with:

- `LOADOUT.md`: the source's name, layer (`layer`) and group (`group`)
- an example skill
- `AGENTS.md`: instructions for AI agents working in the repo
- a CI workflow that runs `lo audit` on every change

Push it to your Git host, then either ask whoever owns the company config to list it
under your team's group, or have people `lo subscribe` to it.

`--upstream` makes your source build on another: anyone who adds yours also
gets the upstream's items, each at its own layer.

## Templates

A template is a skill with blanks, written once by a broader group and filled
in by each team. For example, Engineering publishes "how we work with pull
requests", and each squad fills in its own reviewers and conventions.

**Using one:**

```sh
lo template list --all-sources
lo template show pr-workflow
lo template use pr-workflow --into checkout-squad --set team=Checkout
lo export-source checkout-squad -m "Add our PR workflow"
```

Or on the **Templates** page: pick one, answer the questions, click **Create
skill**. The result is your team's own skill from then on; it remembers which
template it came from.

**Making one:** put it in `templates/<name>/SKILL.md` with `{{blanks}}` and a
`template:` block that describes each blank:

```yaml
---
name: pr-workflow
description: How {{team}} works with pull requests.
template:
  description: A pull request workflow skill; fill in your squad's conventions.
  variables:
    - { name: team, description: Your team or squad }
    - { name: reviewers, default: "2" }
---
# PRs in {{team}}
Every PR needs {{reviewers}} approvals.
```

The web UI's **Make a template** form writes this for you, with a preview
using example answers. Templates are never installed into AI tools
themselves.

## Your own layer

Want your own versions without changing your team's? Create a personal source
(`--layer user`). It outranks every other layer, so your version of any
unlocked skill wins, only for you:

```sh
lo init --new-source my-skills --layer user \
  --upstream https://git.example.com/acme/payments-skills --subscribe
```

## Before you push

```sh
lo audit --path .
```

checks for things that shouldn't be in a shared skill: hidden Unicode,
prompt-injection phrases, dangerous commands and committed secrets. The
scaffolded CI workflow runs the same check on every pull request.

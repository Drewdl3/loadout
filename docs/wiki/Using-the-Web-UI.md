# Using the web UI

`lo ui` opens a dashboard in your browser. It's written for people who
have never used Git or a terminal: everything the CLI can do for your own
setup, you can do here with buttons.

```sh
lo ui                  # opens your browser
lo ui --no-open        # just print the address
lo ui --port 8080      # a fixed port instead of a free one
```

It runs only on your computer (`127.0.0.1`), and the address carries a random
key that changes every time, so other people and other websites can't use it.
Press Ctrl-C in the terminal to stop it.

Every page has an **About this page** note at the top, and **? What do these
words mean** in the sidebar opens a glossary.

## Start here

Where new users land. A three-step explanation of Loadout and a checklist
that follows your real setup:

1. **Connect to your team's skills**: paste your company's link, or add a
   single source.
2. **Choose your AI tools.**
3. **Get the latest skills** (Update now).
4. **Pick the skills you want.**

Each step ticks itself off when it's done.

## Home

Your setup at a glance: how many skills and tools are on, how many sources,
updates waiting for you, and when you last updated. **Your groups** lists
every group in your company config, whether you're in it and how that was
decided, with **Join** / **Leave** buttons for optional groups.

## Browse skills

Everything your groups share, in one list, with a search box and filters by
status, type, layer, source, group, role and tag. Each item has:

- **Turn on** / **Turn off**
- **Use this version**, when two groups at the same rank both have it
- **Join group**, for items from groups you're not in (tick **Include groups
  I'm not in** to see them)
- **Why?**: who provides it and why you have it
- **Edit** and **Copy to another layer…**, for items in sources you have (see
  [Create & edit](#create--edit))

Items show who wrote them (*By Ada, with Grace*), and the **Author** filter
finds everything someone wrote or co-wrote.

## Your layers

The hierarchy view: every skill, tool connection, subagent and rule arranged
by **where it comes from**.

- **Your chain** across the top shows the groups you're in, from the broadest
  (the company) to you. When two have the same thing, the one further right
  wins, unless one to the left has locked it.
- Below it, one card per layer in rank order (company, org, team, product,
  squad, role, you). Inside each, the groups you're in, the sources they
  publish, and their items.
- Each item shows its status and what happened to it:
  - *Replaces the version from org:eng*: this one won over a broader layer.
  - *Replaced by the version from team:payments-dev, which is closer to you*:
    a more specific layer has its own.
  - *Locked here, so layers below can't replace it*.
- The green bar marks what you actually get.
- Filters: one type of item, **Only what I get**, and **Include groups I'm
  not in** to see what joining another group would add.

Use it to answer "which level am I getting this from?" at a glance. See
[How layers work](How-Layers-Work.md) for the rules behind it.

## AI tools

The AI apps Loadout can install into on this computer. Turn on the ones
you use. **Link** (recommended) points each app at Loadout's copy so
updates show up at once; **Copy** puts separate files in the app, for apps
that don't follow links.

## Updates

Some groups ask you to look over their changes before they reach your AI
tools. This page shows what changed in each source (items added, removed,
changed, and anything the safety audit flagged) with an **Approve** button.
See [Updates and review](Updates-and-Review.md).

## Sources

The shared folders your skills come from, drawn as a tree: a source that
builds on another (`upstream:`) is shown under it. Each is marked *from your
company*, *added by you*, or *included by …*. From here you can:

- **See its skills** (opens Browse skills filtered to it)
- **Edit** its items
- **Remove** a source you added yourself
- **Add a source** by its address, with a **Preview** of what it contains
  before you add it

## Templates

Fill-in-the-blanks skills. Pick one, answer a few questions (your team's
name, how many reviewers, …), and you get your own copy in your team's
source. You can also make a new template here. See
[Sharing skills](Sharing-Skills.md#templates).

## Create & edit

- **New skill**: a form that writes the `SKILL.md` for you.
- **Share skills you already have**: finds skills already in your AI tools'
  folders and copies the ones you pick into a source.
- **Edit an item** in any source. The editor has a **Form** for the usual
  fields (description, author, co-authors, tags, mode, roles, locked, the
  instructions) and a **Whole file** view. The form only rewrites the fields
  you change, so comments and other tools' settings in the file stay as they
  are.
- **Copy to another layer…** (also on every item in Browse skills and Your
  layers):
  - **Promote**: copy it to a broader layer's source (say, from your team to
    the whole org) so more people get it. By default it's also removed from
    the original source, since the narrower copy would otherwise keep
    replacing the shared one for that group.
  - **Clone down**: copy it to a layer closer to you (your squad, or your own
    personal source) and change it there. Your version replaces the original
    for that group, unless the original is locked. You're added as a
    co-author.
  - **Turn into a template**: instead of one copy, make a fill-in-the-blanks
    version that each team customizes (see [Templates](#templates)).
- **Open a pull request**: your edits are kept in a private working copy
  until you click this. The source's owners review the pull request, and once
  they merge it, everyone gets the change the next time they update.

Opening a pull request needs a GitHub remote and a GitHub login
(`gh auth login`, or `GH_TOKEN`).

## Health check

Checks this computer for problems: missing AI tools, broken links, two things
with the same name, secrets saved where they shouldn't be. Anything marked
warn or fail says what to do.

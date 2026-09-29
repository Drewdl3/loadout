# How layers work

This page answers "why do I have this skill?", "why didn't I get that one?"
and "why is it the Payments version and not Engineering's?"

## The short version

- You get items from **the sources you're connected to** (and the ones they
  build on), for **the groups you're in**.
- Groups are stacked as **layers**, from the broadest (say, the whole
  company) down to you (most specific).
- When two layers have an item **with the same name**, the one **closer to
  you wins**.
- Except: a layer can **lock** an item, and then nobody below can replace it.
- The ranks come from Loadout's defaults, your sources and your company
  config if you have one, and **you can change them for yourself**.

## A worked example

Jane is in Acme (company), Engineering (org), Payments (team), and has the
developer role. Her layers, broadest first:

```text
acme  →  org: eng  →  team: payments-dev  →  role: developer  →  you
```

| Item | Company | Org: eng | Team: payments-dev | Jane gets |
|---|---|---|---|---|
| `skill/code-review-standards` | ✓ **locked** | | | Company's. Locked, so nobody below can replace it. |
| `skill/write-spec` | | ✓ | ✓ | **Payments'**. Team (rank 20) is closer to Jane than org (rank 10). Engineering's is *Replaced*. |
| `skill/incident-runbook` | | ✓ default-off | | Engineering's, but **off** until she turns it on. |
| `skill/prd-template` | | | | Nothing. It's in the product-manager role's source, and Jane isn't a product manager. |

## See it for yourself

**In the web UI**, open **Your layers**. It shows:

- **Your chain**: the groups you're in, broadest to most specific.
- **Ranks**: every layer with its rank and where the rank came from. Drag a
  layer, or type a rank, to re-rank it for yourself.
- One card per layer, in rank order, with each group you're in and every item
  it shares.
- For each item, its status (**On**, **Off**, **Replaced**) and what happened:
  "Replaces the version from org:eng", "Replaced by the version from
  team:payments-dev, which is closer to you", "Locked here, so layers below
  can't replace it".
- **Why?** on any item opens the full explanation.
- Filters: one type of item, only what you get, or include groups you're not
  in (to see what joining would give you).

**In the terminal:**

```sh
lo why skill/write-spec
```

```text
skill/write-spec — enabled (default)
winner: payments-skills:skill/write-spec (most specific layer)
candidates:
  ✓ payments-skills:skill/write-spec  team:payments-dev (rank 20 from the company config, priority 0) — winner
  ✗ eng-skills:skill/write-spec       org:eng (rank 10 from the company config, priority 0) — outranked by payments-skills:skill/write-spec
targets:
  claude-code: ~/.claude/skills/write-spec (link)
```

Each rank says where it came from: the built-in defaults, a source's
`LOADOUT.md`, the company config, your `[layers]`, or where you (or an
`upstream:` entry) placed that source. The `targets` lines say where the
winning item is installed in each AI tool.

## The rules in full

For each item name (like `skill/write-spec`), Loadout:

1. **Collects** every version from every source you use. Each version has a
   layer and group, from its own `loadout:` block or its source's `LOADOUT.md`,
   and a rank: its layer's rank, unless its source was placed at another
   layer or rank (see [Ranking things yourself](#ranking-things-yourself)).
2. **Keeps only the ones meant for you**: you're in its group, its
   `applies_to` filter (if any) matches you, and it's allowed in the AI tool
   being installed into.
3. **Picks one winner:**
   1. If any version is `locked: true` or `overridable: false`, the
      **broadest** such version wins outright. Attempts to replace it from
      below are ignored, with a warning. Locks are compared at the rank their
      layer was *declared* at, so re-ranking can't get around them.
   2. Otherwise the **most specific** (highest-rank) version wins.
   3. If two versions are at the **same rank**, the one from the source with
      the higher `priority` wins. If that's also tied, it's a **conflict**:
      Loadout picks one (alphabetically, so it's always the same one),
      warns you, and `lo sync` exits with code 4 until you choose with
      `lo prefer <source:kind/name>`.
4. **Decides whether it's on:**
   - `mode: required` or `locked: true`: always on. You can't turn it off.
   - `mode: default-on` (the default): on unless you turn it off.
   - `mode: default-off`: off unless you turn it on.

The resolution never depends on the order sources were fetched in, so two
people with the same groups and the same pinned commits always get the same
result. `lo status` shows a fingerprint (`lo-fp:…`) you can compare.

## Setting an item's layer, mode and audience

In the item's front matter, under `loadout:`:

```yaml
---
name: code-review-standards
description: Review code against Acme's engineering standards.
loadout:
  mode: required          # required | default-on | default-off
  locked: true            # layers below can't replace it
  tags: [review, standards]
---
```

Other fields:

- `layer` / `group`: override the source's layer and group for this one item.
- `overridable: false`: like `locked`, but the item can still be turned off
  unless it's also `required`.
- `applies_to`: narrow the audience further, for example
  `applies_to: { role: [product-manager] }`. Every axis listed must match;
  values within an axis are alternatives.
- `targets`: only install into these AI tools, for example `[claude-code]`.

The full format is in [`docs/agents.md`](../agents.md) and
[`schemas/item-block.schema.json`](../../schemas/item-block.schema.json).

## Where ranks come from

Each layer's rank is merged from, later ones winning:

1. Loadout's built-in ranks (`company` 0, `org` 10, `team` 20, `product`
   25, `squad` 30, `project` 35, `role` 40, `user` 50);
2. the `layers:` of your sources' `LOADOUT.md` files, so a team can bring
   a layer of its own, like `{ name: beta, rank: 27 }` (if two sources
   disagree, the higher rank is used, with a warning);
3. the company config's layers, if you have one;
4. your own `[layers]` in `config.toml`.

`lo layers` lists them all:

```text
Layers, broadest first (higher rank wins):
  company      0  from the built-in defaults
  org         10  from the built-in defaults
  team        20  from the built-in defaults
  beta        27  from beta-skills's LOADOUT.md
  …
  user        50  from the built-in defaults
```

## Ranking things yourself

Everything about ranks can be changed for yourself, without asking anyone:

```sh
lo layers set beta=15                     # re-rank a whole layer
lo layers unset beta                      # back to its declared rank
lo source set beta-skills --rank 35       # rank one source's items at 35
lo source set beta-skills --layer product # …or at the product layer's rank
lo source set beta-skills --label "Beta (payments)"
lo source set payments-skills --priority 10   # wins ties at the same rank
lo source unset beta-skills --rank --label
```

A source's `--layer` or `--rank` applies to all of its items, even those
that name their own layer. It works for any source you get: ones you
subscribed to, upstreams, and ones your company config lists (unless the
company config sets `allow_local_ranks: false`). A team can do the same for
its subscribers in its `upstream:` entries; your own setting wins.

In the web UI, use **Place…** on the **Sources** page, and **Ranks** on
**Your layers**. Changes apply straight away.

**Locks don't move.** A locked item is compared at the rank its source
declared, whatever you've re-ranked. Moving the company's source below your
team doesn't let the team replace a locked company skill, and moving some
other source above the company doesn't let *its* lock win. `lo why` says
so when that happens.

## Your own layer

A **personal source** (`layer: user`) is a layer just for you, and it outranks
every other one. Put your own version of an unlocked skill there and yours
wins. Create one with:

```sh
lo init --new-source my-skills --layer user --upstream https://git.example.com/acme/payments-skills --subscribe
```

## Common questions

**I'm in a group but don't get its skill.** Check `lo why kind/name`. The
usual reasons: the item has an `applies_to` that doesn't match you, it's
`default-off`, a locked version from a broader layer wins, or it's limited to
other AI tools with `targets`.

**I want the Engineering version, not the team's.** Re-rank Engineering's
source above the team's for yourself: `lo source set eng-skills --rank 25`
(or **Place…** on the **Sources** page). Between versions at the *same*
rank, choose with `lo prefer` (or **Use this version** in the UI).

**I can't turn something off.** It's `required` or `locked`. Those are for
things like security rules; ask the source's owners.

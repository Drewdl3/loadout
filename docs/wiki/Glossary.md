# Glossary

Where the CLI or the file formats use another word, it's in brackets.

| Word | Meaning |
|---|---|
| **Skill** | Written instructions an AI assistant follows for a particular job. A folder with a `SKILL.md`. |
| **Tool connection** (MCP server) | Lets an AI assistant use another system, such as Jira or a database. |
| **Subagent** (agent) | A helper assistant with its own instructions that your main assistant can hand work to. |
| **Rule or prompt** (extra) | Shorter instructions: house rules, saved prompts, slash commands. |
| **Item** | Any of the above. Named `kind/name`; its full id is `source:kind/name`. |
| **Source** | A Git repo of items, usually one per team, with a `LOADOUT.md` at the top. Connect to one with `lo init <link>`. |
| **Upstream** | A source that another source builds on. Adding the second gets you the first too. |
| **Nested source** | A source in a folder of another source's repo, with its own `LOADOUT.md` (listed under `nested:`). It comes with the repo; you get its items if you're in its group. |
| **Group** | A set of people: a team, an org, a role, the company, just you. |
| **Layer** | A level of your organization, such as company, org, team, squad or role, with a rank: higher rank = more specific = wins ties. Ranks come from the defaults, your sources' `layers:`, the company config, and your own `[layers]`, in that order; `lo layers` shows them. Your personal `user` layer ranks highest unless something ranks it. |
| **Placement** | Where a source sits for you: its label, the layer or exact rank its items get, and its priority for ties (`lo source set`, **Place…**). |
| **Declared rank** | The rank a layer's sources or company config gave it, before you re-ranked it. Locks are compared at this rank. |
| **Your chain** | The groups you're in, from the broadest to you, shown on **Your layers**. |
| **Company config** | Optional. A list of layers, groups, their sources, who belongs to each, and policy, which sets people up automatically. |
| **Locked** | A layer's item that no layer below can replace or turn off, however anyone re-ranks things. |
| **Required** | An item that's always on. |
| **Replaced** (shadowed) | A version that lost to one from a layer closer to you. |
| **Conflict** | Two versions at the same rank with nothing to choose between them. Settle it with `lo prefer`. |
| **AI tool** (target) | An AI app Loadout installs into, such as Claude Code or Cursor. |
| **Update** (sync) | Fetch the latest from your sources and install it. |
| **Pending** | An update waiting for you to review and approve it. |
| **Lock file** (`loadout.lock`) | The exact commit of every source and hash of every item you have. |
| **Fingerprint** (`lo-fp:…`) | A short summary of your whole setup; equal fingerprints mean identical setups. |
| **Shortcode** (`lo1_…`) | Your exact setup as a string someone else can import. |
| **Template** | A skill with `{{blanks}}` that each team fills in to get its own copy. |
| **Working copy** | Your private clone of a source where edits wait until you open a pull request. |
| **Pull request** | A proposed change the source's owners review before it reaches everyone. |
| **Project mode** | Items for one code repo, installed into that repo and pinned in its own `loadout.lock`. |

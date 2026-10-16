# Managing the skills on your computer

Loadout doesn't only install what your team shares. It can also see every
skill your AI tools have: ones you wrote yourself, ones another installer
(such as `npx skills`) put there, ones in the project you're working on.
It can then take care of them for you.

## See everything

In the web UI, open **My computer**. In a terminal:

```sh
lo skills                 # every skill, who installed it, what to fix
lo skills --issues        # only the ones worth fixing
lo skills --in ~/work/app # also look in a project's folders
```

For each skill you see:

- **Whether Loadout manages it.** A managed skill comes from one of your
  sources; the rest are *not managed*.
- **Which AI tools have it.** A folder that other tools link to counts once.
  For example, `npx skills` keeps a skill in `~/.agents/skills` and links it
  into Pi's folder, so that skill is one copy seen by Codex and Pi.
- **Where it came from**, when its installer recorded that. `npx skills`
  does, so Loadout knows `tdd` came from `acme/skills` on GitHub.
- **What's worth fixing:**
  - *Not in Claude Code*: a tool you use doesn't have it.
  - *2 copies, and they differ*: there's more than one copy, and they're not the same.
  - *Claude Code sees more than one copy*: for example, a global one and a
    project one.
  - *Your own copy is in the way*: Loadout couldn't install your team's
    version because a skill of yours has the same name.

## Manage a skill

Click **Manage**, or run `lo skills manage <name>`. To manage everything
that isn't managed yet, use **Manage all** or `lo skills manage --all`.
Loadout then:

1. **Follows the repo the skill came from**, when its installer recorded
   one. The repo becomes one of your sources, with only the skills you had
   turned on, so you keep getting its updates. It doesn't need a
   `LOADOUT.md`: any repo of skill folders works. The page groups these
   skills, so one click on **Follow acme/skills** handles all of them.
2. **Otherwise, copies it into your local skills**: a private source on
   this computer (it's called `local-skills`, shown as *This computer*).
   You can edit these skills in the web UI. **Save & install** applies a
   change straight away, without a pull request.
3. **Installs it into every AI tool you use**, and moves your original
   folders (and other tools' links to them) to a backup in Loadout's data
   folder. Each tool ends up with exactly one copy.

Prefer a copy you control over following the repo? Click **Copy instead**,
or run `lo skills manage <name> --copy`.

If a skill's copies aren't all the same, Loadout won't guess which one to
keep. Open the skill's copies, click **Keep this one** on the right one, or
run `lo skills manage <name> --path <folder>`. The other copies stay where
they are for you to sort out.

Skills in a project folder are left alone: they belong to that project's
repo. To share one with your team, use
[Share skills you already have](Sharing-Skills.md).

## Stop managing a skill

Click **Stop managing**, or run `lo skills unmanage <name>`. Loadout stops
installing the skill (it drops your local copy, or turns the followed one
off) and puts your original folders and links back where they were.

## Following any skills repo

You don't need an installer to follow a repo of skills: paste its link
into **Paste a link** (or run `lo init <url>` or `lo subscribe <url>`).
A repo without a `LOADOUT.md` is read as a plain skills repo:

- it's named after its URL (`acme/skills` becomes `acme-skills`);
- every folder with a `SKILL.md`, at any depth, is a skill;
- it sits at the `user` layer.

You can't edit a followed repo through Loadout, because there's nowhere to
send the changes. To change one of its skills, use **Copy to another
layer…** to make your own version.

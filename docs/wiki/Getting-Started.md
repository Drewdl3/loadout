# Getting started

This page gets your AI tools set up with your company's and team's skills.
It takes about five minutes. You don't need to know Git.

## 1. Install `lo`

**Linux or macOS (x64):**

```sh
curl -fsSL https://raw.githubusercontent.com/Drewdl3/loadout/main/install.sh | sh
```

**Windows (x64), in PowerShell:**

```powershell
irm https://raw.githubusercontent.com/Drewdl3/loadout/main/install.ps1 | iex
```

The scripts download the latest release, check it against the published
checksums, and put `lo` on your `PATH`.

**Other machines (e.g. ARM):** install [Rust](https://rustup.rs) 1.96 or
newer, then:

```sh
cargo install --git https://github.com/Drewdl3/loadout loadout-cli
```

You also need `git`. Loadout uses the Git logins you already have, so if
you can clone your company's repos, Loadout can too.

Check it worked:

```sh
lo --version
```

Later, `lo self-update` updates to the newest release.

## 2. Connect

You have two options. Pick whichever you like; they do the same thing.

### Option A: the web UI (no terminal after this)

```sh
lo ui
```

Your browser opens on **Start here**. Click **I have my company's link**,
paste the link your company gave you (it looks like
`https://git.example.com/acme/agent-config`), and click **Connect**.
Loadout then:

- works out which groups you're in (your org, your team, your role),
- finds the AI tools installed on this computer,
- downloads your groups' skills and installs them.

The checklist on the page ticks itself off as you go. See
[Using the web UI](Using-the-Web-UI.md) for a tour of every page.

### Option B: the terminal

```sh
lo init https://git.example.com/acme/agent-config
```

It asks which AI tools to set up, then does the same as above.

### No company link?

Your company might not use a *company config* (the list of groups). You can still add
a team's source directly:

```sh
lo subscribe https://git.example.com/acme/payments-skills
lo sync
```

In the web UI, go to **Sources** and paste the address under **Add a source**.
You can preview what's in it first.

## 3. Check what you got

```sh
lo list                    # everything installed
lo profile                 # the groups you're in, and how that was decided
```

Or open **Your layers** in the web UI to see every skill arranged by the group
it comes from, from the whole company down to you. See
[How layers work](How-Layers-Work.md).

Now open your AI tool (for example Claude Code) and the skills are there.

## 4. Stay up to date

```sh
lo sync                    # fetch and install the latest; safe to run any time
lo schedule enable         # or let the OS do it every hour
```

In the web UI, click **Update now**. Some groups ask you to look over their
changes before they reach your tools; those show up under **Updates**. See
[Updates and review](Updates-and-Review.md).

## Next steps

- Turn skills on or off: **Browse skills** in the web UI, or
  `lo enable <id>` / `lo disable <id>`.
- Join an optional group: `lo join product:billing`, or **Join** on the
  **Home** page.
- Share a skill of your own: [Sharing skills](Sharing-Skills.md).

## Try it without touching your setup

The repo's [`examples/`](../../examples/README.md) folder has a complete
fictional company (Acme). `examples/try.sh` runs it in a throwaway sandbox,
so nothing touches your real home directory or AI tools.

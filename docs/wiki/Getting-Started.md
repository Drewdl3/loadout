# Getting started

This page gets your AI tools set up with your team's skills. It takes about
five minutes. You don't need to know Git.

Start with your team's source (or your own), and connect to the ones above
it when you need them. Often your team's source already does that for you.

## 1. Install `lo`

**Linux or macOS (x64 or arm64):**

```sh
curl -fsSL https://raw.githubusercontent.com/Drewdl3/loadout/main/install.sh | sh
```

**Windows (x64), in PowerShell:**

```powershell
irm https://raw.githubusercontent.com/Drewdl3/loadout/main/install.ps1 | iex
```

The scripts download the latest release, check it against the published
checksums, and put `lo` on your `PATH`.

**Other machines:** install [Rust](https://rustup.rs) 1.96 or
newer, then:

```sh
cargo install --git https://github.com/Drewdl3/loadout loadout-cli
```

You also need `git`. Loadout uses the Git logins you already have, so if
you can clone your team's repos, Loadout can too.

Check it worked:

```sh
lo --version
```

Later, `lo self-update` updates to the newest release.

## 2. Connect to your team's skills

### Got a link?

Someone on your team gave you a link to their source, a Git repo like
`https://git.example.com/acme/payments-skills`. That's all you need.

**In the web UI** (no terminal after this):

```sh
lo ui
```

Your browser opens on **Start here**. Click **Paste a link**, paste it, and
click **Connect**. **In the terminal:**

```sh
lo init https://git.example.com/acme/payments-skills
```

Either way, Loadout:

- looks inside and shows you what you'd get, including the sources it
  builds on (its *upstreams*, such as your org's),
- subscribes you to it, which puts you in its group,
- finds the AI tools installed on this computer,
- downloads the skills and installs them.

The same works if the link is to a *company config* (a list of groups
some companies keep): Loadout then works out which groups you're in and
sets you up for all of them. See
[Growing across your org](Growing-Across-Your-Org.md).

To add more sources later, use **Sources** in the web UI (paste the address
under **Add a source**, and preview it first) or `lo subscribe <link>`.

### No link yet? Start your own

Start a source for your team, or just for yourself. It takes a minute, and
you can connect it to other teams' sources later.

**In the web UI**, click **Start your own** on **Start here**. **In the
terminal**, run `lo init` with no arguments and pick an option, or:

```sh
lo init --new-source ~/loadout/payments-skills --layer team --group payments-dev --subscribe
lo init --new-source ~/loadout/my-skills --layer user --subscribe    # just for you
```

Push the folder to your Git host and share its link; your teammates connect
with `lo init <link>`. [Sharing skills](Sharing-Skills.md) covers writing
the skills themselves.

## 3. Check what you got

```sh
lo list                    # everything installed
lo profile                 # the groups you're in, and how that was decided
```

Or open **Your layers** in the web UI to see every skill arranged by the group
it comes from, from the broadest down to you. See
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
- Get another team's skills: `lo subscribe <link>`, or **Sources** in the
  web UI. Choose where they rank for you with `lo source set <name> --rank N`.
- Share a skill of your own: [Sharing skills](Sharing-Skills.md).

## Try it without touching your setup

The repo's [`examples/`](../../examples/README.md) folder has a complete
fictional company (Acme), with team sources connected through `upstream:`
and a company config on top. `examples/try.sh` runs it in a throwaway sandbox,
so nothing touches your real home directory or AI tools.

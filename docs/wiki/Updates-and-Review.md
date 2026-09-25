# Updates and review

Instructions for AI assistants are powerful, so Loadout treats updates
carefully: everything is pinned, checked, and, unless your company says
otherwise, shown to you before it's installed.

## What `lo sync` does

1. **Fetch** the latest from every source.
2. **Plan**: work out what would change for you (items added, removed or
   changed).
3. **Audit** the changes (see below).
4. **Apply** the changes your company auto-applies (usually the `company`
   layer). Everything else **waits for your review**.

It's always safe to run. If nothing changed, nothing happens.

## Reviewing

```sh
lo status                   # 1 change pending: payments-skills a1b2c3d4 → e5f6a7b8 (needs review)
lo diff payments-skills     # what changed, audit findings, the text diff
lo approve payments-skills  # install it
lo approve --all
```

In the web UI: the **Updates** page, with a count in the sidebar.

`lo sync --yes` approves everything pending in one go (audit blocks still
apply).

## Which updates wait

- Layers listed in the company config's `policy.auto_apply` install straight away.
- Everything else waits, including every source you added yourself. You can
  auto-apply your own list in `config.toml`:
  ```toml
  [policy]
  auto_apply = ["team"]
  ```

## The safety audit

Every change is scanned for:

- hidden or look-alike Unicode characters,
- prompt-injection phrases ("ignore previous instructions"…),
- dangerous commands (piping downloads into a shell, deleting everything…),
- committed tokens and keys.

A **high** finding holds the change even for auto-applied layers. A
**critical** one (like a committed token) blocks it until you approve with
`--force-audit`. The rules live in [`audit/`](../../audit/README.md). Treat
the audit as a speed bump: pinning and review are the real protection.

## Pinning and reproducibility

Every source is pinned to an exact commit in `loadout.lock`, with a hash of
every item. That means:

- `lo sync --locked` installs exactly what the lock says, with no
  fetching. Good for CI and a second machine.
- `lo status` shows a fingerprint (`lo-fp:…`). Two people with the same
  fingerprint have exactly the same setup.
- `lo export` gives you a short code (`lo1_…`) for your exact setup.
  Someone else runs `lo import <code>` to get the same thing, even if the
  sources have moved on since. It never contains secrets.

## Signed commits

A company config can require that some layers only accept commits signed by listed
SSH or GPG keys (`policy.require_signed` and `signers`). An unsigned or
wrongly signed commit is refused and the previous version is kept.

## Keeping up to date automatically

```sh
lo schedule enable              # every hour
lo schedule enable --interval 4h
lo schedule status
lo schedule disable
```

This uses your OS's own scheduler (launchd on macOS, a systemd user timer or
cron on Linux, Task Scheduler on Windows). There's no background service.
Pending reviews still wait for you.

# Browser tests for `lo ui`

Each test builds a sandboxed computer: its own home folder with fake AI tool
folders, local Git repos standing in for remote ones, and its own `lo ui`.
Then it clicks through the web UI the way a person would. Nothing touches
your real home or AI tools.

```sh
cargo build -p loadout-cli        # the tests run target/debug/lo (or $LO_BIN)
cd e2e
npm ci
npx playwright install chromium   # once
npx playwright test               # desktop + phone
npx playwright test --headed      # watch it
```

- `screenshots/` keeps a screenshot of each step worth looking at,
  `<project>-<name>.png`, including every page at desktop and phone width
  (`tests/tour.spec.ts`). Look through them after UI changes.
- The tour fails if a page scrolls sideways or logs an error.
- A failing test leaves a trace in `test-results/`:
  `npx playwright show-trace <trace.zip>`.
- `KEEP_SANDBOX=1` keeps each test's sandbox folder for poking around.

`support/world.ts` has the sandbox (`World`) and ready-made setups, such as
`npxSkillsComputer` (skills installed by `npx skills`, linked into several
tools, plus one of your own) and `teamSource` (a team source, subscribed and
synced).

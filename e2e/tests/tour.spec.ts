// A screenshot of every page in a realistic setup, at desktop and phone
// width (`e2e/screenshots/`), checking nothing overflows sideways and no
// page logs an error. Look through them after UI changes.

import { expect, npxSkillsComputer, open, teamSource, test } from "../support/world";

const PAGES = ["Start here", "Home", "Browse skills", "My computer", "Your layers", "AI tools", "Updates", "Sources", "Templates", "Share setup", "Create & edit", "Health check"];

test("every page renders @tour", async ({ world, ui, shot, isMobile }) => {
  npxSkillsComputer(world);
  teamSource(world);
  for (const label of PAGES) {
    if (isMobile) {
      // The sidebar may be behind a menu on small screens.
      const menu = ui.getByRole("button", { name: /menu/i });
      if (await menu.isVisible()) await menu.click();
    }
    await open(ui, label);
    await expect(ui.locator("#main")).not.toContainText("Loading…");
    const overflow = await ui.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow, `${label} scrolls sideways`).toBeLessThanOrEqual(1);
    await shot("page-" + label.toLowerCase().replace(/[^a-z]+/g, "-"));
  }
});

// "Share skills you already have": pick, review (rename, tag, …), add to a
// source, open one pull request.

import * as fs from "node:fs";
import * as path from "node:path";
import { expect, npxSkillsComputer, open, skill, teamSource, test } from "../support/world";

test("review step: tag them all, rename one, add to the team's source", async ({ world, ui, shot }) => {
  npxSkillsComputer(world);
  world.write(world.at(".claude/skills/Meeting_Notes/SKILL.md"), skill("meeting-notes", "Summarise a meeting."));
  teamSource(world);
  await open(ui, "Create & edit");
  await ui.getByRole("button", { name: "Find my skills…" }).click();
  const drawer = ui.locator("#drawer");
  await expect(drawer.getByLabel("tdd")).toBeVisible();
  // A name that isn't kebab-case can still be picked: it's renamed on review.
  await drawer.getByLabel("Meeting_Notes").check();
  await drawer.getByLabel("tdd").check();
  await drawer.getByRole("button", { name: "Next: review" }).click();
  await expect(drawer.getByText("Review 2 skills")).toBeVisible();
  await expect(drawer.getByLabel("Name").first()).toHaveValue("meeting-notes");
  await drawer.getByLabel("Tags for all").fill("community");
  await drawer.getByLabel("Author for all").fill("Ada Lovelace");
  await drawer.getByRole("button", { name: "Apply to all" }).click();
  await expect(drawer.getByLabel("Tags", { exact: true }).nth(1)).toHaveValue("community");
  await shot("adopt-review");
  await drawer.getByRole("button", { name: "Add to source" }).last().click();
  await expect(drawer.getByText("Added to team-skills:")).toBeVisible();
  await expect(drawer.getByRole("button", { name: "Open pull request" })).toBeVisible();
  await shot("adopt-done");

  const work = fs.readdirSync(path.join(world.data, "work")).map(d => path.join(world.data, "work", d));
  const text = fs.readFileSync(path.join(work[0], "skills/meeting-notes/SKILL.md"), "utf8");
  expect(text).toContain("name: meeting-notes");
  expect(text).toContain('tags: ["community"]');
  expect(text).toContain('author: "Ada Lovelace"');
  // Adopted skills are marked, not offered twice.
  await drawer.getByRole("button", { name: "Look" }).click();
  await expect(drawer.getByLabel("tdd")).toBeDisabled();
  await expect(drawer).toContainText("adopted into team-skills");
});

// "My computer": every skill on the machine, and managing the ones Loadout
// didn't install.

import * as fs from "node:fs";
import * as path from "node:path";
import { expect, linkTarget, npxSkillsComputer, open, skill, teamSource, test, toast } from "../support/world";

const row = (page: import("@playwright/test").Page, name: string) => page.locator(`[data-skill="${name}"]`);

test("shows every skill, where it came from, and what to fix", async ({ world, ui, shot }) => {
  npxSkillsComputer(world);
  await open(ui, "My computer");
  await expect(ui.locator(".tile").first()).toContainText("3");
  // npx skills recorded the repo: offer to follow it.
  await expect(ui.getByText("2 skills from acme/skills")).toBeVisible();
  await expect(ui.getByRole("button", { name: "Follow acme/skills" })).toBeVisible();
  const tdd = row(ui, "tdd");
  await expect(tdd).toContainText("not managed");
  await expect(tdd).toContainText("from acme/skills");
  await expect(tdd).toContainText("Not in Claude Code");
  await expect(row(ui, "my-notes")).toContainText("Not in Codex, Pi");
  await shot("computer-before");
});

test("follows the repo in one click and keeps one copy", async ({ world, ui, shot }) => {
  npxSkillsComputer(world);
  await open(ui, "My computer");
  await ui.getByRole("button", { name: "Follow acme/skills" }).click();
  await toast(ui, "Now managing grill-me, tdd");
  await expect(row(ui, "tdd")).toContainText("managed");
  await expect(row(ui, "tdd")).toContainText("via acme-skills");
  await expect(row(ui, "tdd").getByRole("button", { name: "Stop managing" })).toBeVisible();
  // Every tool now has Loadout's copy; the original is backed up.
  const store = path.join(world.data, "store");
  for (const p of [".claude/skills/tdd", ".agents/skills/tdd", ".pi/agent/skills/tdd"]) {
    expect(linkTarget(world.at(p)), p).toContain(store);
  }
  expect(fs.existsSync(path.join(world.data, "adopted/tdd/tests.md"))).toBe(true);
  // The repo's other skill isn't installed.
  expect(fs.existsSync(world.at(".claude/skills/to-spec"))).toBe(false);
  await shot("computer-after-follow");
});

test("manage all, then edit your own skill and install it", async ({ world, ui, shot }) => {
  npxSkillsComputer(world);
  await open(ui, "My computer");
  await ui.getByRole("button", { name: "Manage all 3" }).click();
  await toast(ui, "Now managing");
  await expect(row(ui, "my-notes")).toContainText("your local skills");
  await row(ui, "my-notes").getByRole("button", { name: "Edit" }).click();
  const drawer = ui.locator("#drawer");
  await expect(drawer).toContainText("Edit skill/my-notes");
  await drawer.getByLabel("Description").fill("How I take meeting notes, with action items. Use when summarising a meeting.");
  await drawer.getByLabel("Tags").fill("notes, meetings");
  await drawer.getByRole("button", { name: "Save changes" }).click();
  await expect(drawer.getByRole("button", { name: "Save & install" })).toBeVisible();
  await shot("editor-local-skill");
  await drawer.getByRole("button", { name: "Save & install" }).click();
  await toast(ui, "Saved and installed");
  const installed = fs.readFileSync(world.at(".claude/skills/my-notes/SKILL.md"), "utf8");
  expect(installed).toContain("with action items");
  expect(installed).toContain('tags: ["notes","meetings"]');
});

test("stop managing puts the original back", async ({ world, ui }) => {
  npxSkillsComputer(world);
  world.json("skills", "manage", "tdd");
  await open(ui, "My computer");
  ui.once("dialog", d => d.accept());
  await row(ui, "tdd").getByRole("button", { name: "Stop managing" }).click();
  await toast(ui, "Put your tdd back");
  await expect(row(ui, "tdd")).toContainText("not managed");
  expect(linkTarget(world.at(".agents/skills/tdd"))).toBeNull();
  expect(fs.existsSync(world.at(".agents/skills/tdd/tests.md"))).toBe(true);
  expect(linkTarget(world.at(".pi/agent/skills/tdd"))).toBe(world.at(".agents/skills/tdd"));
});

test("copies that differ: pick the one to keep", async ({ world, ui, shot }) => {
  npxSkillsComputer(world);
  world.write(world.at(".claude/skills/tdd/SKILL.md"), skill("tdd", "My own take on test-first."));
  await open(ui, "My computer");
  const tdd = row(ui, "tdd");
  await expect(tdd).toContainText("2 copies on this computer, and they're not the same.");
  // No one-click manage: you choose. Bulk actions leave it out too.
  await expect(tdd.getByRole("button", { name: "Manage", exact: true })).toHaveCount(0);
  await expect(ui.getByText("1 skill from acme/skills")).toBeVisible();
  await expect(ui.getByRole("button", { name: "Manage all 2" })).toBeVisible();
  await tdd.locator("summary").click();
  await shot("computer-differing-copies");
  const keep = tdd.locator("div.row", { hasText: "~/.agents/skills/tdd" }).getByRole("button", { name: "Keep this one" }).first();
  await keep.click();
  await toast(ui, "Now managing tdd");
  // Your other copy is still there, and in Loadout's way in Claude Code.
  await expect(row(ui, "tdd")).toContainText("Your own copy is in the way");
});

test("a project's skills show up too", async ({ world, ui }) => {
  npxSkillsComputer(world);
  const project = path.join(world.root, "work/app");
  fs.mkdirSync(path.join(project, ".git"), { recursive: true });
  world.write(path.join(project, ".claude/skills/my-notes/SKILL.md"), skill("my-notes", "The project's notes skill."));
  await open(ui, "My computer");
  await ui.getByLabel("Project folder").fill(project);
  await ui.getByRole("button", { name: "Look" }).click();
  await expect(row(ui, "my-notes")).toContainText("Claude Code sees more than one copy.");
});

test("skills from your team's source can be turned off, not 'unmanaged'", async ({ world, ui }) => {
  npxSkillsComputer(world);
  teamSource(world);
  await open(ui, "My computer");
  const spec = row(ui, "write-spec");
  await expect(spec).toContainText("via team-skills");
  await expect(spec.getByRole("button", { name: "Stop managing" })).toHaveCount(0);
  await spec.getByRole("button", { name: "Turn off" }).click();
  await toast(ui, "Turned off write-spec");
  expect(fs.existsSync(world.at(".claude/skills/write-spec"))).toBe(false);
});

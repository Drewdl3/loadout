// A sandboxed computer for one test: its own home directory (with fake AI
// tool folders), Loadout config and data folders, local Git repos standing
// in for remote ones, and its own `lo ui`. Nothing touches your real home.

import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import * as readline from "node:readline";
import { test as base, expect, type Page } from "@playwright/test";

const REPO = path.resolve(__dirname, "../..");
/** The `lo` binary: `LO_BIN`, else the debug build (`cargo build -p loadout-cli`). */
const LO = process.env.LO_BIN ?? path.join(REPO, "target/debug/lo");
/** Screenshots worth looking at, kept across runs: `e2e/screenshots/`. */
export const SHOTS = path.resolve(__dirname, "../screenshots");

export type Run = { code: number; stdout: string; stderr: string };

export class World {
  readonly root: string;
  readonly home: string;
  readonly config: string;
  readonly data: string;
  readonly env: NodeJS.ProcessEnv;
  private ui?: ChildProcess;

  constructor() {
    if (!fs.existsSync(LO)) throw new Error(`${LO} not found: run \`cargo build -p loadout-cli\` (or set LO_BIN)`);
    this.root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "lo-e2e-")));
    this.home = path.join(this.root, "home");
    this.config = path.join(this.root, "config");
    this.data = path.join(this.root, "data");
    const scratch = path.join(this.root, "scratch");
    for (const d of [this.home, scratch]) fs.mkdirSync(d, { recursive: true });
    const gitconfig = path.join(scratch, "gitconfig");
    fs.writeFileSync(gitconfig, "[user]\n\tname = Ada Lovelace\n\temail = ada@example.com\n[init]\n\tdefaultBranch = main\n");
    // The same isolation the Rust tests use (crates/loadout-cli/tests/common).
    this.env = {
      PATH: process.env.PATH,
      HOME: this.home,
      LOADOUT_HOME: this.home,
      LOADOUT_CONFIG_DIR: this.config,
      LOADOUT_DATA_DIR: this.data,
      XDG_CONFIG_HOME: path.join(this.home, ".config"),
      XDG_DATA_HOME: path.join(this.home, ".local/share"),
      GIT_CONFIG_NOSYSTEM: "1",
      GIT_CONFIG_GLOBAL: gitconfig,
      LOADOUT_TEST_KEYSTORE: path.join(this.root, "keystore.json"),
      LOADOUT_RELEASES_API: "http://127.0.0.1:9",
      LOADOUT_GITHUB_API: "http://127.0.0.1:9",
    };
  }

  /** `~/<rel>`. */
  at(rel: string): string {
    return path.join(this.home, rel);
  }

  lo(...args: string[]): Run {
    const r = spawnSync(LO, args, { cwd: this.home, env: this.env, encoding: "utf8" });
    return { code: r.status ?? 1, stdout: r.stdout, stderr: r.stderr };
  }

  /** `lo <args> --json`, which must succeed. */
  json(...args: string[]): any {
    const r = this.lo(...args, "--json");
    if (r.code !== 0) throw new Error(`lo ${args.join(" ")} failed (${r.code}):\n${r.stdout}\n${r.stderr}`);
    return JSON.parse(r.stdout);
  }

  git(cwd: string, ...args: string[]): string {
    const r = spawnSync("git", args, { cwd, env: this.env, encoding: "utf8" });
    if (r.status !== 0) throw new Error(`git ${args.join(" ")}: ${r.stderr}`);
    return r.stdout;
  }

  write(file: string, text: string) {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, text);
  }

  link(at: string, to: string) {
    fs.mkdirSync(path.dirname(at), { recursive: true });
    fs.symlinkSync(to, at);
  }

  /** A Git repo under `<root>/remotes/<name>` with `files`, committed. */
  repo(name: string, files: Record<string, string>): string {
    const dir = path.join(this.root, "remotes", name);
    fs.mkdirSync(dir, { recursive: true });
    this.git(dir, "init", "--quiet");
    for (const [rel, text] of Object.entries(files)) this.write(path.join(dir, rel), text);
    this.git(dir, "add", "--all");
    this.git(dir, "commit", "--quiet", "-m", "init");
    return dir;
  }

  /** Makes an AI tool look installed. */
  tool(...dirs: string[]) {
    for (const d of dirs) fs.mkdirSync(this.at(d), { recursive: true });
  }

  /** Starts `lo ui` and returns its URL (with the token). */
  async startUi(): Promise<string> {
    const child = spawn(LO, ["ui", "--no-open", "--json"], { cwd: this.home, env: this.env, stdio: ["ignore", "pipe", "pipe"] });
    this.ui = child;
    const stderr: string[] = [];
    child.stderr!.on("data", d => stderr.push(String(d)));
    const line = await new Promise<string>((resolve, reject) => {
      const rl = readline.createInterface({ input: child.stdout! });
      rl.once("line", resolve);
      child.once("exit", code => reject(new Error(`lo ui exited (${code}): ${stderr.join("")}`)));
    });
    return JSON.parse(line).url;
  }

  dispose() {
    this.ui?.kill();
    if (!process.env.KEEP_SANDBOX) fs.rmSync(this.root, { recursive: true, force: true });
  }
}

export function skill(name: string, description: string, body = `Do the ${name} thing.`): string {
  return `---\nname: ${name}\ndescription: ${description}\n---\n\n${body}\n`;
}

/** Where a link points (absolute), or null if it isn't one. */
export function linkTarget(p: string): string | null {
  try {
    return path.resolve(path.dirname(p), fs.readlinkSync(p));
  } catch {
    return null;
  }
}

/**
 * The setup people end up with after `npx skills add acme/skills -g`:
 * Claude Code, Codex and Pi installed; tdd and grill-me as real folders in
 * `~/.agents/skills` (Codex's folder), linked into Pi's, with the lock file
 * naming the repo; plus a skill of your own in Claude Code's folder.
 */
export function npxSkillsComputer(w: World): { repo: string } {
  w.tool(".claude", ".codex", ".pi/agent");
  const repo = w.repo("acme-skills", {
    "README.md": "# Acme's skills\n",
    "skills/engineering/tdd/SKILL.md": skill("tdd", "Write the test first. Use when adding a feature or fixing a bug."),
    "skills/engineering/tdd/tests.md": "What a good test looks like.\n",
    "skills/writing/grill-me/SKILL.md": skill("grill-me", "Ask hard questions about a plan. Use before building."),
    "skills/writing/to-spec/SKILL.md": skill("to-spec", "Turn a chat into a spec."),
  });
  const agents = w.at(".agents/skills");
  w.write(path.join(agents, "tdd/SKILL.md"), skill("tdd", "Write the test first. Use when adding a feature or fixing a bug."));
  w.write(path.join(agents, "tdd/tests.md"), "What a good test looks like.\n");
  w.write(path.join(agents, "grill-me/SKILL.md"), skill("grill-me", "Ask hard questions about a plan. Use before building."));
  w.link(w.at(".pi/agent/skills/tdd"), "../../../.agents/skills/tdd");
  w.link(w.at(".pi/agent/skills/grill-me"), "../../../.agents/skills/grill-me");
  const entry = (n: string, p: string) => ({ source: "acme/skills", sourceType: "github", sourceUrl: repo, skillPath: p });
  w.write(w.at(".agents/.skill-lock.json"), JSON.stringify({
    version: 3,
    skills: { tdd: entry("tdd", "skills/engineering/tdd/SKILL.md"), "grill-me": entry("grill-me", "skills/writing/grill-me/SKILL.md") },
  }));
  w.write(w.at(".claude/skills/my-notes/SKILL.md"), skill("my-notes", "How I take meeting notes. Use when summarising a meeting."));
  return { repo };
}

/** A team source (with `LOADOUT.md`), subscribed and synced. */
export function teamSource(w: World): string {
  const url = w.repo("team-skills", {
    "LOADOUT.md": "---\nloadout: 1\nname: team-skills\nlayer: team\ngroup: payments-dev\n---\n# Payments team\n",
    "skills/write-spec/SKILL.md": skill("write-spec", "Write a spec the Payments way. Use when asked for a spec."),
  });
  w.json("subscribe", url);
  w.json("sync");
  return url;
}

type Fixtures = { world: World; ui: Page; shot: (name: string) => Promise<void>; consoleErrors: string[] };

export const test = base.extend<Fixtures>({
  world: async ({}, use) => {
    const w = new World();
    await use(w);
    w.dispose();
  },
  consoleErrors: async ({}, use) => {
    await use([]);
  },
  // A page on the world's `lo ui`. Call `world` setup before using it:
  // the UI starts on first use.
  ui: async ({ page, world, consoleErrors }, use) => {
    page.on("console", m => { if (m.type() === "error") consoleErrors.push(m.text()); });
    page.on("pageerror", e => consoleErrors.push(String(e)));
    const url = await world.startUi();
    await page.goto(url);
    await use(page);
    expect(consoleErrors, "the page logged errors").toEqual([]);
  },
  // Saves a full-page screenshot to e2e/screenshots/<project>-<name>.png.
  shot: async ({ page }, use, info) => {
    fs.mkdirSync(SHOTS, { recursive: true });
    await use(async name => {
      await page.screenshot({ path: path.join(SHOTS, `${info.project.name}-${name}.png`), fullPage: true });
    });
  },
});

export { expect };

/** Opens a page from the sidebar. */
export async function open(page: Page, label: string) {
  await page.getByRole("navigation").getByRole("button", { name: label, exact: false }).first().click();
  await expect(page.locator("#main h1")).toBeVisible();
}

/** Waits for a toast saying `text`. */
export async function toast(page: Page, text: string | RegExp) {
  await expect(page.locator("#toast")).toContainText(text);
}

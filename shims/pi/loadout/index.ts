// Loadout extension for Pi: `/lo <command>` and sync on session start.
// Install: copy this directory to ~/.pi/agent/extensions/loadout/
// (or run `pi --extension ./shims/pi/loadout/index.ts`).
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { execFile } from "node:child_process";

/** Runs `lo <args> --json --exit-zero` and resolves with its stdout. */
function loadout(args: string[]): Promise<string> {
  return new Promise((resolve) => {
    execFile(
      "lo",
      [...args, "--json", "--exit-zero"],
      { maxBuffer: 16 * 1024 * 1024 },
      (error, stdout, stderr) => {
        if (error && !stdout) {
          resolve(JSON.stringify({ error: String(stderr || error.message) }));
        } else {
          resolve(stdout);
        }
      },
    );
  });
}

/** Splits `why skill/x --kind mcp` into arguments (quotes supported). */
function splitArgs(input: string): string[] {
  const out: string[] = [];
  const re = /"([^"]*)"|'([^']*)'|(\S+)/g;
  for (const m of input.matchAll(re)) out.push(m[1] ?? m[2] ?? m[3]);
  return out;
}

export default function (pi: ExtensionAPI) {
  pi.on("session_start", async () => {
    execFile("lo", ["sync", "--if-stale", "--quiet", "--non-interactive", "--exit-zero"], () => {});
  });

  pi.registerCommand("lo", {
    description: "Run a Loadout command (why, list, status, diff, enable, disable…) and explain the result",
    handler: async (args, ctx) => {
      const argv = splitArgs(String(args ?? "").trim());
      if (argv.length === 0) {
        ctx.ui.notify("usage: /lo <command> [args], e.g. /lo why skill/write-spec", "info");
        return;
      }
      const output = await loadout(argv);
      pi.sendUserMessage(
        `Here is the output of \`lo ${argv.join(" ")}\` (Loadout manages my agent configuration). ` +
          "Explain it briefly; don't run further commands unless I ask.\n\n```json\n" +
          output.trim() +
          "\n```",
      );
    },
  });
}

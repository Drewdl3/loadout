import { defineConfig, devices } from "@playwright/test";

// Every test starts its own `lo ui` against a fresh sandbox home (see
// support/world.ts), so tests run in parallel and never touch your real
// AI tool folders.
export default defineConfig({
  testDir: "tests",
  timeout: 60_000,
  expect: { timeout: 10_000 },
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : [["list"]],
  use: {
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 860 } } },
    // The tour again at phone width: nothing should overflow.
    { name: "phone", use: { ...devices["Pixel 7"] }, grep: /@tour/ },
  ],
});

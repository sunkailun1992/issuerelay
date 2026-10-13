import { defineConfig } from "@playwright/test";

const PORT = 4173;

/**
 * The screenshots of the panel, and the layout checks that need a real browser. They run on this
 * machine only, never in CI: the panel's fonts are the system's, so the same page looks a little
 * different on every machine and the comparison would not hold on a CI runner. What it guards
 * against is an unintended change to a page on the same machine, so whoever changes the panel
 * runs `pnpm --dir app test:screens` before committing, and `pnpm --dir app screenshots` to
 * accept the change on purpose.
 *
 * The panel is built and served by `vite preview`, with the mock backend's fictional data (the
 * address picks the scenario and the language, see `src/lib/api.ts`). The browser is the Google
 * Chrome installed on the machine (`channel: "chrome"`), so no browser has to be downloaded.
 */
export default defineConfig({
  testDir: "./tests",
  testMatch: "*.spec.ts",
  // The baselines are kept with the other acceptance material, named `<language>-<theme>-<page>.png`.
  snapshotPathTemplate: "../docs/gates/screenshots/m2/{arg}{ext}",
  fullyParallel: true,
  reporter: [["list"], ["html", { open: "never" }]],
  use: {
    baseURL: `http://localhost:${PORT}`,
    channel: "chrome",
    viewport: { width: 1100, height: 720 },
    deviceScaleFactor: 1,
    // Dates and times in the panel are shown in the browser's time zone.
    locale: "en-US",
    timezoneId: "UTC",
  },
  projects: [
    { name: "light", use: { colorScheme: "light" } },
    { name: "dark", use: { colorScheme: "dark" } },
  ],
  webServer: {
    command: "pnpm build && pnpm preview --port 4173",
    url: `http://localhost:${PORT}`,
    // Always the build this run makes, never a server that was left running.
    reuseExistingServer: false,
    timeout: 180_000,
  },
});

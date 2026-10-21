import { defineConfig } from "playwright/test";

const executablePath = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE;

export default defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: false,
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:4174",
    browserName: "chromium",
    launchOptions: executablePath ? { executablePath } : undefined,
  },
  webServer: [
    {
      command: "node tests/fixtures/server.mjs",
      url: "http://127.0.0.1:4175/admin/",
      reuseExistingServer: false,
    },
    {
      command: "pnpm dev --host 127.0.0.1 --port 4174",
      url: "http://127.0.0.1:4174/admin/",
      reuseExistingServer: false,
    },
  ],
});

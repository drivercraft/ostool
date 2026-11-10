import { defineConfig } from "playwright/test";
import { randomUUID } from "node:crypto";
import { join } from "node:path";
import { tmpdir } from "node:os";
const runId = randomUUID().replaceAll("-", "");
process.env.OSTOOL_E2E_LOCAL_DATABASE_SCHEMA ??= "ostool_e2e_local_" + runId;
process.env.OSTOOL_E2E_LOCAL_AUTH_STATE ??= join(
  tmpdir(),
  "ostool-e2e-auth-" + runId + ".json",
);

const executablePath = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE;
const publicFixture = process.env.OSTOOL_E2E_PUBLIC === "true";
if (publicFixture) {
  process.env.OSTOOL_E2E_AUTH_BASE_URL = "http://127.0.0.1:4191";
  process.env.OSTOOL_E2E_AUTH_DATABASE_URL =
    process.env.OSTOOL_E2E_LOCAL_DATABASE_URL;
  process.env.OSTOOL_E2E_AUTH_DATABASE_SCHEMA ??=
    "ostool_e2e_security_" + randomUUID().replaceAll("-", "");
  process.env.OSTOOL_E2E_ADMIN_EMAIL ??= "browser-admin@test.local";
  process.env.OSTOOL_E2E_ADMIN_PASSWORD ??= "browser-password-123";
}

export default defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: false,
  workers: 1,
  globalTeardown: "./tests/fixtures/cleanup.mjs",
  projects: [
    {
      name: "authenticate",
      testMatch: /local-auth.setup.ts/,
      use: { storageState: undefined },
    },
    {
      name: "browser",
      testIgnore: /local-auth.setup.ts/,
      dependencies: ["authenticate"],
      use: { storageState: process.env.OSTOOL_E2E_LOCAL_AUTH_STATE },
    },
  ],
  use: {
    baseURL: "http://127.0.0.1:4174",
    browserName: "chromium",
    launchOptions: executablePath ? { executablePath } : undefined,
  },
  webServer: [
    ...(publicFixture
      ? [
          {
            command: "node tests/fixtures/public-server.mjs",
            url: "http://127.0.0.1:4191/api/v1/auth/config",
            reuseExistingServer: false,
          },
        ]
      : []),
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

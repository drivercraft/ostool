// Public browser security fixture; never connects to a deployment schema.
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
const schema = process.env.OSTOOL_E2E_AUTH_DATABASE_SCHEMA;
const database = process.env.OSTOOL_E2E_AUTH_DATABASE_URL;
if (
  !schema?.startsWith("ostool_e2e_") ||
  !database ||
  !["127.0.0.1", "localhost", "[::1]"].includes(new URL(database).hostname)
)
  throw new Error(
    "Public browser fixture requires an isolated loopback test database",
  );
const root = await mkdtemp(join(tmpdir(), "ostool-public-e2e-"));
const child = spawn(
  process.env.OSTOOL_TEST_SERVER_BIN ||
    resolve("../../target/debug/ostool-server"),
  ["--env-file", "/dev/null"],
  {
    stdio: "inherit",
    env: {
      ...process.env,
      OSTOOL_LISTEN_ADDR: "127.0.0.1:4191",
      OSTOOL_DATA_DIR: root + "/data",
      OSTOOL_NETWORK_INTERFACE: "lo",
      OSTOOL_TFTP_ENABLED: "false",
      OSTOOL_NETWORK_TEST_ENABLED: "false",
      OSTOOL_LOADER_NETWORK_ENABLED: "false",
      OSTOOL_LOADER_BIND_ADDR: "127.0.0.1:2998",
      OSTOOL_HTTP_BOOT_ENABLED: "true",
      OSTOOL_HTTP_BOOT_BASE_URL: "http://127.0.0.1:4191",

      OSTOOL_EDITION: "public",
      OSTOOL_REDIS_URL: process.env.OSTOOL_E2E_REDIS_URL,
      OSTOOL_DATABASE_URL: database,
      OSTOOL_DATABASE_SCHEMA: schema,
      OSTOOL_PUBLIC_BASE_URL: "http://127.0.0.1:4191",
      OSTOOL_ADMIN_EMAIL: process.env.OSTOOL_E2E_ADMIN_EMAIL,
      OSTOOL_ADMIN_PASSWORD: process.env.OSTOOL_E2E_ADMIN_PASSWORD,
      OSTOOL_MFA_ENCRYPTION_KEY: randomBytes(32).toString("base64"),
    },
  },
);
let stopping = false;
for (const signal of ["SIGTERM", "SIGINT"])
  process.on(signal, () => {
    stopping = true;
    child.kill("SIGTERM");
    setTimeout(() => child.kill("SIGKILL"), 3000).unref();
  });
child.on("error", async (error) => {
  console.error(error);
  await rm(root, { recursive: true, force: true });
  process.exit(1);
});
child.on("exit", async (code) => {
  await rm(root, { recursive: true, force: true });
  process.exit(stopping ? 0 : (code ?? 1));
});

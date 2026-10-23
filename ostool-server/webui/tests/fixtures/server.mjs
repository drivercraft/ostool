// A private server instance. No system config, service or board is touched.
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
const root = await mkdtemp(join(tmpdir(), "ostool-admin-e2e-"));
const binary =
  process.env.OSTOOL_TEST_SERVER_BIN ||
  resolve("../../target/debug/ostool-server");
if (!process.env.OSTOOL_E2E_LOCAL_DATABASE_URL)
  throw new Error(
    "Set OSTOOL_E2E_LOCAL_DATABASE_URL to a disposable PostgreSQL database for browser tests",
  );
const child = spawn(binary, ["--env-file", "/dev/null"], {
  stdio: "inherit",
  env: {
    ...process.env,
    OSTOOL_LISTEN_ADDR: "127.0.0.1:4175",
    OSTOOL_DATA_DIR: root + "/data",
    OSTOOL_NETWORK_INTERFACE: "lo",
    OSTOOL_TFTP_ENABLED: "false",
    OSTOOL_NETWORK_TEST_ENABLED: "false",
    OSTOOL_LOADER_NETWORK_ENABLED: String(
      process.env.OSTOOL_TEST_DISABLE_DISCOVERY !== "true",
    ),
    OSTOOL_LOADER_BIND_ADDR: "127.0.0.1:2998",
    OSTOOL_HTTP_BOOT_ENABLED: "true",
    OSTOOL_HTTP_BOOT_BASE_URL: "http://127.0.0.1:4175",

    OSTOOL_EDITION: "local",
    OSTOOL_DATABASE_URL: process.env.OSTOOL_E2E_LOCAL_DATABASE_URL,
    OSTOOL_DATABASE_SCHEMA: `ostool_e2e_${randomUUID().replaceAll("-", "")}`,
  },
});
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

// Public browser security fixture; never connects to a deployment schema.
import { mkdtemp, writeFile, rm } from "node:fs/promises";
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
await writeFile(
  join(root, "config.toml"),
  `
listen_addr = "127.0.0.1:4191"
data_dir = "${root}/data"
[tftp]
provider = "builtin"
enabled = false
root_dir = "${root}/tftp"
bind_addr = "127.0.0.1:0"
[network]
interface = "lo"
[network_test]
enabled = false
[loader_network]
enabled = false
bind_addr = "127.0.0.1:2998"
`,
);
const child = spawn(
  process.env.OSTOOL_TEST_SERVER_BIN ||
    resolve("../../target/debug/ostool-server"),
  ["--config", join(root, "config.toml")],
  {
    stdio: "inherit",
    env: {
      ...process.env,
      OSTOOL_EDITION: "public",
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

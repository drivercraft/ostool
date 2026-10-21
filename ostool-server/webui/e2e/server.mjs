// A private server instance. No system config, service or board is touched.
import { mkdtemp, writeFile, rm, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { spawn } from "node:child_process";
const root = await mkdtemp(join(tmpdir(), "ostool-admin-e2e-"));
await writeFile(
  join(root, "config.toml"),
  `
listen_addr = "127.0.0.1:4175"
data_dir = "${root}/data"
board_dir = "${root}/boards"
dtb_dir = "${root}/dtbs"
[tftp]
provider = "builtin"
enabled = false
root_dir = "${root}/tftp"
bind_addr = "127.0.0.1:0"
[network]
interface = "lo"
[http_boot]
enabled = true
root_dir = "${root}/httpboot"
public_base_url = "http://127.0.0.1:4175"
[loader_network]
enabled = ${process.env.OSTOOL_TEST_DISABLE_DISCOVERY !== "true"}
bind_addr = "127.0.0.1:2998"
public_base_url = "http://127.0.0.1:4175"
`,
);
await mkdir(join(root, "boards"), { recursive: true });
await writeFile(
  join(root, "boards", "legacy.toml"),
  "id = 'legacy'\nboard_type = 'old'\n[boot]\nkind = 'removed-version'\n",
);
const binary =
  process.env.OSTOOL_TEST_SERVER_BIN ||
  resolve("../../target/debug/ostool-server");
const child = spawn(binary, ["--config", join(root, "config.toml")], {
  stdio: "inherit",
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

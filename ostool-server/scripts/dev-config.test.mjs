import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  mkdtempSync,
  readFileSync,
  existsSync,
  writeFileSync,
  rmSync,
  mkdirSync,
  copyFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { createServer } from "node:net";
import {
  developmentEnvironment,
  developmentHost,
  developmentNetwork,
  developmentCaddyfile,
} from "./dev-config.mjs";

test("default dotenv is resolved beside the server directory, not the caller cwd", () => {
  const root = mkdtempSync(join(tmpdir(), "ostool-dev-default-env-"));
  try {
    const server = join(root, "ostool-server");
    mkdirSync(join(server, "scripts"), { recursive: true });
    for (const file of ["dev.sh", "dev-config.mjs"])
      copyFileSync(
        resolve(import.meta.dirname, file),
        join(server, "scripts", file),
      );
    writeFileSync(
      join(server, ".env"),
      "OSTOOL_EDITION=local\nOSTOOL_DEV_HOST=192.168.20.11\nOSTOOL_DEV_SERVER_PORT=4197\n",
    );
    // A different dotenv in the caller's cwd must never override the server file.
    writeFileSync(
      join(root, ".env"),
      "OSTOOL_EDITION=public\nOSTOOL_DEV_SERVER_PORT=9999\n",
    );
    const env = Object.fromEntries(
      Object.entries(process.env).filter(([key]) => !key.startsWith("OSTOOL_")),
    );
    const result = spawnSync(
      "bash",
      [join(server, "scripts/dev.sh"), "status"],
      { env, cwd: root, encoding: "utf8" },
    );
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /edition: local/);
    assert.match(result.stdout, /requested listener: 192\.168\.20\.11:4197/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("dotenv values are literal and support quotes/comments without shell execution", () => {
  const env = developmentEnvironment(
    `
export OSTOOL_DEV_HOST="192.168.20.10" # LAN
OSTOOL_DEV_DIR='/tmp/development files'
OSTOOL_ADMIN_PASSWORD='$(touch /tmp/must-never-execute)\`whoami\` # literal'
OSTOOL_DEV_DEMO=false
`,
    {},
  );
  assert.equal(env.OSTOOL_DEV_HOST, "192.168.20.10");
  assert.equal(env.OSTOOL_DEV_DIR, "/tmp/development files");
  assert.equal(
    env.OSTOOL_ADMIN_PASSWORD,
    "$(touch /tmp/must-never-execute)`whoami` # literal",
  );
  assert.equal(env.OSTOOL_DEV_DEMO, "false");
});

test("explicit process values including empty overrides win without mutating their input", () => {
  const processEnv = { OSTOOL_DEV_HOST: "", OSTOOL_EDITION: "public" };
  const env = developmentEnvironment(
    "OSTOOL_DEV_HOST=192.168.20.10\nOSTOOL_EDITION=local\nOSTOOL_DEV_DEMO=false",
    processEnv,
  );
  assert.equal(env.OSTOOL_DEV_HOST, "");
  assert.equal(env.OSTOOL_EDITION, "public");
  assert.equal(env.OSTOOL_DEV_DEMO, "false");
  assert.deepEqual(processEnv, {
    OSTOOL_DEV_HOST: "",
    OSTOOL_EDITION: "public",
  });
});

test("deployment credentials, proxy policy and executable configuration are not imported", () => {
  const env = developmentEnvironment(
    `
OSTOOL_DATABASE_URL=postgres://deployment/production
OSTOOL_REDIS_URL=redis://deployment
OSTOOL_LISTEN_ADDR=203.0.113.1:80
OSTOOL_TRUSTED_PROXY_IPS=203.0.113.2
NODE_OPTIONS=--require=/tmp/untrusted.cjs
PATH=/tmp/untrusted
OSTOOL_DEV_DB_NAME=development
`,
    {},
  );
  for (const key of [
    "OSTOOL_DATABASE_URL",
    "OSTOOL_REDIS_URL",
    "OSTOOL_LISTEN_ADDR",
    "OSTOOL_TRUSTED_PROXY_IPS",
    "NODE_OPTIONS",
    "PATH",
  ])
    assert.equal(Object.hasOwn(env, key), false);
  assert.equal(env.OSTOOL_DEV_DB_NAME, "development");
});

test("launcher applies dotenv before resolving edition/listener and works from another cwd", () => {
  const root = mkdtempSync(join(tmpdir(), "ostool-dev-config-"));
  try {
    const file = join(root, "dev.env");
    writeFileSync(
      file,
      "OSTOOL_EDITION=local\nOSTOOL_DEV_HOST=192.168.20.10\nOSTOOL_DEV_SERVER_PORT=4195\nOSTOOL_DEV_DIR='/tmp/development files'\n",
    );
    const env = Object.fromEntries(
      Object.entries(process.env).filter(([key]) => !key.startsWith("OSTOOL_")),
    );
    env.OSTOOL_DEV_ENV_FILE = file;
    const script = resolve(import.meta.dirname, "dev.sh");
    const result = spawnSync("bash", [script, "status"], {
      env,
      cwd: root,
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /edition: local/);
    assert.match(result.stdout, /requested listener: 192\.168\.20\.10:4195/);
    assert.match(result.stdout, /state: \/tmp\/development files/);
    const override = spawnSync("bash", [script, "status"], {
      env: {
        ...env,
        OSTOOL_DEV_HOST: "127.0.0.1",
        OSTOOL_DEV_SERVER_PORT: "4196",
      },
      cwd: root,
      encoding: "utf8",
    });
    assert.equal(override.status, 0, override.stderr);
    assert.match(override.stdout, /requested listener: 127\.0\.0\.1:4196/);
    const missing = spawnSync("bash", [script, "status"], {
      env: { ...env, OSTOOL_DEV_ENV_FILE: join(root, "missing.env") },
      encoding: "utf8",
    });
    assert.equal(missing.status, 1);
    assert.match(missing.stderr, /failed to load development environment/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("build uses the repository root, propagates failures and never starts services", () => {
  const root = mkdtempSync(join(tmpdir(), "ostool-dev-build-"));
  try {
    const bin = join(root, "bin");
    mkdirSync(bin);
    const log = join(root, "cargo.log");
    const envFile = join(root, "dev.env");
    const state = join(root, "unused-state");
    writeFileSync(envFile, `OSTOOL_EDITION=local\nOSTOOL_DEV_DIR='${state}'\n`);
    writeFileSync(
      join(bin, "cargo"),
      `#!/usr/bin/env bash
printf '%s\\n' "$PWD" "$@" > "$OSTOOL_TEST_BUILD_LOG"
[[ -z "\u0024{OSTOOL_SERVER_WEB_DIST_DIR+x}" ]] || exit 56
exit "\u0024{OSTOOL_TEST_BUILD_EXIT:-0}"
`,
      { mode: 0o755 },
    );
    writeFileSync(join(bin, "docker"), "#!/usr/bin/env bash\nexit 57\n", {
      mode: 0o755,
    });
    const env = {
      ...process.env,
      PATH: bin + ":" + process.env.PATH,
      OSTOOL_DEV_ENV_FILE: envFile,
      OSTOOL_DEV_DIR: state,
      OSTOOL_TEST_BUILD_LOG: log,
      OSTOOL_SERVER_WEB_DIST_DIR: "/tmp/stale-prebuilt-assets",
    };
    delete env.OSTOOL_DEV_CONFIG_LOADED;
    const script = resolve(import.meta.dirname, "dev.sh");
    const result = spawnSync("bash", [script, "build"], {
      env,
      cwd: root,
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(
      readFileSync(log, "utf8"),
      resolve(import.meta.dirname, "../..") + "\nbuild\n-p\nostool-server\n",
    );
    assert.equal(
      existsSync(state),
      false,
      "build must not create database/config/PID state",
    );
    assert.match(result.stdout, /build complete; use restart/);
    const failed = spawnSync("bash", [script, "build"], {
      env: { ...env, OSTOOL_TEST_BUILD_EXIT: "42" },
      cwd: root,
      encoding: "utf8",
    });
    assert.equal(failed.status, 42);
    assert.doesNotMatch(failed.stdout, /build complete/);
    assert.equal(existsSync(state), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("start rejects an occupied port before launching database services", async () => {
  const root = mkdtempSync(join(tmpdir(), "ostool-dev-busy-port-"));
  const server = createServer();
  try {
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(0, "127.0.0.1", resolve);
    });
    const port = server.address().port;
    const bin = join(root, "bin");
    mkdirSync(bin);
    const marker = join(root, "docker-called");
    writeFileSync(
      join(bin, "docker"),
      `#!/usr/bin/env bash
printf called > "$OSTOOL_TEST_DOCKER_MARKER"
exit 69
`,
      { mode: 0o755 },
    );
    for (const tool of ["cargo", "pnpm"])
      writeFileSync(join(bin, tool), "#!/usr/bin/env bash\nexit 0\n", {
        mode: 0o755,
      });
    const file = join(root, "dev.env");
    writeFileSync(
      file,
      `OSTOOL_EDITION=local\nOSTOOL_DEV_HOST=127.0.0.1\nOSTOOL_DEV_SERVER_PORT=${port}\nOSTOOL_DEV_DIR='${join(root, "state")}'\n`,
    );
    const env = Object.fromEntries(
      Object.entries(process.env).filter(([key]) => !key.startsWith("OSTOOL_")),
    );
    Object.assign(env, {
      PATH: bin + ":" + process.env.PATH,
      OSTOOL_DEV_ENV_FILE: file,
      OSTOOL_TEST_DOCKER_MARKER: marker,
    });
    const result = spawnSync(
      "bash",
      [resolve(import.meta.dirname, "dev.sh"), "start"],
      { env, encoding: "utf8" },
    );
    assert.equal(result.status, 1, result.stdout + result.stderr);
    assert.match(result.stderr, /cannot bind 127\.0\.0\.1:/);
    assert.match(result.stderr, /another edition\/process/);
    assert.equal(existsSync(marker), false);
    assert.equal(existsSync(join(root, "state/server.pid")), false);
  } finally {
    server.close();
    rmSync(root, { recursive: true, force: true });
  }
});

test("development launchers reject public IPs before creating state or starting services", () => {
  const root = mkdtempSync(join(tmpdir(), "ostool-dev-public-host-"));
  try {
    const bin = join(root, "bin");
    mkdirSync(bin);
    const marker = join(root, "docker-called");
    writeFileSync(
      join(bin, "docker"),
      '#!/usr/bin/env bash\nprintf called > "$OSTOOL_TEST_DOCKER_MARKER"\nexit 69\n',
      { mode: 0o755 },
    );
    for (const tool of ["cargo", "pnpm"])
      writeFileSync(join(bin, tool), "#!/usr/bin/env bash\nexit 0\n", {
        mode: 0o755,
      });
    const env = Object.fromEntries(
      Object.entries(process.env).filter(([key]) => !key.startsWith("OSTOOL_")),
    );
    for (const host of ["203.0.113.10", "8.8.8.8", "2001:db8::10"]) {
      const state = join(root, "state");
      const result = spawnSync(
        "bash",
        [resolve(import.meta.dirname, "dev.sh"), "start"],
        {
          env: {
            ...env,
            PATH: bin + ":" + process.env.PATH,
            OSTOOL_DEV_CONFIG_LOADED: "1",
            OSTOOL_EDITION: "public",
            OSTOOL_DEV_DEMO: "true",
            OSTOOL_DEV_HOST: host,
            OSTOOL_DEV_DIR: state,
            OSTOOL_TEST_DOCKER_MARKER: marker,
          },
          encoding: "utf8",
        },
      );
      assert.equal(result.status, 1, result.stdout + result.stderr);
      assert.match(result.stderr, /仅允许本机、局域网或通配地址/);
      assert.equal(existsSync(marker), false);
      assert.equal(existsSync(state), false);
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("both development editions support private listeners and optional isolated HTTPS ingress", () => {
  for (const host of [
    "127.0.0.1",
    "0.0.0.0",
    "192.168.1.10",
    "10.3.10.160",
    "172.16.1.2",
    "169.254.1.2",
    "::1",
    "::",
    "fd00::1",
  ]) {
    assert.doesNotThrow(() => developmentNetwork(host, "2999"));
  }
  for (const host of [
    "8.8.8.8",
    "2001:4860:4860::8888",
    "example.com",
    "192.168.1.1\nadmin off",
  ])
    assert.equal(developmentHost(host), false);
  assert.doesNotThrow(() =>
    developmentNetwork("127.0.0.1", "2999", "10.3.10.160", "3443"),
  );
  assert.throws(() =>
    developmentNetwork("0.0.0.0", "2999", "10.3.10.160", "3443"),
  );
  assert.throws(() =>
    developmentNetwork("127.0.0.1", "2999", "0.0.0.0", "3443"),
  );
  assert.throws(() =>
    developmentNetwork("127.0.0.1", "2999", "10.3.10.160", "70000"),
  );
  const caddy = developmentCaddyfile("10.3.10.160", "3443", "2999");
  assert.match(caddy, /https:\/\/10\.3\.10\.160:3443/);
  assert.match(caddy, /bind 10\.3\.10\.160/);
  assert.match(caddy, /tls internal/);
  assert.match(caddy, /admin off/);
  assert.match(caddy, /reverse_proxy 127\.0\.0\.1:2999/);
  assert.match(caddy, /respond @hardware 404/);
});

test("public HTTPS isolates the backend while local edition ignores public proxy options", () => {
  const root = mkdtempSync(join(tmpdir(), "ostool-dev-edition-https-"));
  try {
    const file = join(root, "dev.env");
    writeFileSync(
      file,
      "OSTOOL_EDITION=public\nOSTOOL_DEV_HOST=0.0.0.0\nOSTOOL_DEV_HTTPS_HOST=192.168.20.10\nOSTOOL_DEV_HTTPS_PORT=3443\n",
    );
    const bin = join(root, "bin");
    mkdirSync(bin);
    writeFileSync(join(bin, "docker"), "#!/usr/bin/env bash\nexit 1\n", {
      mode: 0o755,
    });
    const env = Object.fromEntries(
      Object.entries(process.env).filter(([key]) => !key.startsWith("OSTOOL_")),
    );
    Object.assign(env, {
      PATH: bin + ":" + process.env.PATH,
      OSTOOL_DEV_ENV_FILE: file,
    });
    const script = resolve(import.meta.dirname, "dev.sh");
    const https = spawnSync("bash", [script, "status"], {
      env,
      encoding: "utf8",
    });
    assert.equal(https.status, 0, https.stderr);
    assert.match(https.stdout, /requested listener: 127\.0\.0\.1:2999/);
    assert.match(https.stdout, /HTTPS: https:\/\/192\.168\.20\.10:3443/);
    const local = spawnSync("bash", [script, "status"], {
      env: { ...env, OSTOOL_EDITION: "local" },
      encoding: "utf8",
    });
    assert.equal(local.status, 0, local.stderr);
    assert.match(local.stdout, /requested listener: 0\.0\.0\.0:2999/);
    assert.doesNotMatch(local.stdout, /HTTPS:/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

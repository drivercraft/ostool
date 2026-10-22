import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  mkdtempSync,
  writeFileSync,
  rmSync,
  mkdirSync,
  copyFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { developmentEnvironment } from "./dev-config.mjs";

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

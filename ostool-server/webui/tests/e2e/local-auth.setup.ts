import { test, expect } from "playwright/test";
import { createHash } from "node:crypto";
import pg from "pg";
test("authenticate the isolated administrator", async ({ request }) => {
  const base = "http://127.0.0.1:4175";
  const schema = process.env.OSTOOL_E2E_LOCAL_DATABASE_SCHEMA!;
  const database = process.env.OSTOOL_E2E_LOCAL_DATABASE_URL!;
  if (
    !/^ostool_e2e_[a-z0-9_]+$/.test(schema) ||
    !["localhost", "127.0.0.1", "[::1]"].includes(new URL(database).hostname)
  )
    throw new Error("setup requires disposable loopback storage");
  const captcha = await request.get(
    base + "/api/v1/auth/captcha?action=admin_login",
  );
  expect(captcha.ok()).toBeTruthy();
  const { id } = await captcha.json();
  const client = new pg.Client({ connectionString: database });
  await client.connect();
  try {
    await client.query(
      `UPDATE "${schema}".security_challenges SET secret_hash=$1 WHERE id=$2`,
      [
        createHash("sha256")
          .update(`ostool:captcha:admin_login:${id}:AB29Q`)
          .digest("hex"),
        id,
      ],
    );
  } finally {
    await client.end();
  }
  const response = await request.post(base + "/api/v1/admin/auth/login", {
    headers: { origin: base },
    data: {
      email: "local-browser-admin@test.local",
      password: "local-browser-password-123",
      verification: { captcha_id: id, captcha_code: "AB29Q" },
    },
  });
  expect(response.status(), await response.text()).toBe(200);
  await request.storageState({
    path: process.env.OSTOOL_E2E_LOCAL_AUTH_STATE!,
  });
});

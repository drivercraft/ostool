// Tests solve real image challenges through an explicitly isolated, loopback database.
// The application has no captcha bypass endpoint or deployment switch.
import { createHash } from "node:crypto";
import pg from "pg";
import { expect, type APIRequestContext, type Page } from "playwright/test";

export async function securityDb<T>(
  work: (client: pg.Client) => Promise<T>,
): Promise<T> {
  const connectionString = process.env.OSTOOL_E2E_AUTH_DATABASE_URL;
  const schema = process.env.OSTOOL_E2E_AUTH_DATABASE_SCHEMA;
  const base = new URL(
    process.env.OSTOOL_E2E_AUTH_BASE_URL ?? "http://invalid",
  );
  if (!connectionString || !schema || !/^ostool_e2e_[a-z0-9_]+$/.test(schema))
    throw new Error(
      "Public security tests require an explicit disposable ostool_e2e_ schema",
    );
  const database = new URL(connectionString);
  if (
    ![base.hostname, database.hostname].every((host) =>
      ["127.0.0.1", "localhost", "[::1]"].includes(host),
    )
  )
    throw new Error(
      "Security test fixtures only accept loopback servers/databases",
    );
  const client = new pg.Client({ connectionString });
  await client.connect();
  try {
    await client.query('SET search_path TO "' + schema + '"');
    return await work(client);
  } finally {
    await client.end();
  }
}
async function knownAnswer(id: string): Promise<void> {
  await securityDb(async (client) => {
    const row = await client.query(
      "SELECT purpose FROM security_challenges WHERE id=$1",
      [id],
    );
    expect(row.rows).toHaveLength(1);
    const hash = createHash("sha256")
      .update("ostool:" + row.rows[0].purpose + ":" + id + ":AB29Q")
      .digest("hex");
    await client.query(
      "UPDATE security_challenges SET secret_hash=$2 WHERE id=$1",
      [id, hash],
    );
  });
}
export async function solveCaptcha(page: Page): Promise<void> {
  const widget = page.locator(".human-verification[data-captcha-id]");
  await expect(widget).toHaveAttribute("data-captcha-id", /[a-f0-9-]{36}/);
  await knownAnswer((await widget.getAttribute("data-captcha-id"))!);
  await page.getByLabel("图形验证码", { exact: true }).fill("AB29Q");
}
async function proof(request: APIRequestContext, base: string, action: string) {
  const response = await request.get(
    base + "/api/v1/auth/captcha?action=" + action,
  );
  expect(response.ok()).toBeTruthy();
  const challenge = await response.json();
  await knownAnswer(challenge.id);
  return { captcha_id: challenge.id, captcha_code: "AB29Q" };
}
export async function apiLogin(
  request: APIRequestContext,
  base: string,
  email: string,
  password: string,
  admin = true,
) {
  const response = await request.post(
    base + "/api/v1/" + (admin ? "admin/" : "") + "auth/login",
    {
      headers: { origin: base },
      data: {
        email,
        password,
        verification: await proof(
          request,
          base,
          admin ? "admin_login" : "login",
        ),
      },
    },
  );
  expect(response.status(), await response.text()).toBe(200);
  expect((await response.json()).access_token).toBeUndefined();
  const prefix = admin ? "ostool-admin-access=" : "ostool-website-access=";
  const cookie = response
    .headersArray()
    .find(
      (header) =>
        header.name.toLowerCase() === "set-cookie" &&
        header.value.startsWith(prefix),
    );
  expect(cookie).toBeTruthy();
  return { accessToken: cookie!.value.slice(prefix.length).split(";")[0] };
}
export async function apiRegister(
  request: APIRequestContext,
  base: string,
  email: string,
  password: string,
  display_name: string,
) {
  return request.post(base + "/api/v1/auth/register", {
    headers: { origin: base },
    data: {
      email,
      password,
      display_name,
      legal:{accepted:true,terms_version:1,privacy_version:1},
      verification: await proof(request, base, "register"),
    },
  });
}

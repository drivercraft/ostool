import { createHmac, createHash } from "node:crypto";
import { expect, test } from "playwright/test";
import { solveCaptcha, securityDb } from "../fixtures/security";

function totp(secret: string) {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  const bits = [...secret]
    .map((char) => alphabet.indexOf(char).toString(2).padStart(5, "0"))
    .join("");
  const key = Buffer.from(
    bits.match(/.{8}/g)!.map((byte) => parseInt(byte, 2)),
  );
  const step = Buffer.alloc(8);
  step.writeBigInt64BE(BigInt(Math.floor(Date.now() / 30000)));
  const digest = createHmac("sha1", key).update(step).digest();
  return String(
    (digest.readUInt32BE(digest[19] & 15) & 0x7fffffff) % 1000000,
  ).padStart(6, "0");
}
test("image captcha, encrypted MFA enrollment, recovery login and recent identity confirmation", async ({
  page,
  context,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires a disposable PostgreSQL public server");
  test.setTimeout(60_000);
  const email = process.env.OSTOOL_E2E_ADMIN_EMAIL!;
  const password = process.env.OSTOOL_E2E_ADMIN_PASSWORD!;
  const errors: string[] = [];
  const events: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => {
    if (request.url().includes("/events")) events.push(request.url());
  });
  await page.goto(base + "/admin/login?redirect=/admin/account");
  await expect(
    page.getByRole("button", { name: "登录", exact: true }),
  ).toBeDisabled();
  await page.getByLabel("邮箱", { exact: true }).fill(email);
  await page.getByLabel("密码", { exact: true }).fill(password);
  await solveCaptcha(page);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(page).toHaveURL(base + "/admin/account");
  await page
    .getByRole("button", { name: "设置身份验证器", exact: true })
    .click();
  await expect(page.getByAltText("身份验证器设置二维码")).toBeVisible();
  const secret = await page.locator(".mfa-secret").innerText();
  await page.getByLabel("身份验证器验证码", { exact: true }).fill(totp(secret));
  await page.getByRole("button", { name: "确认启用", exact: true }).click();
  await expect(page.locator(".recovery-codes code")).toHaveCount(8);
  const recovery = await page.locator(".recovery-codes code").allTextContents();
  expect(recovery).toHaveLength(8);
  await page.getByRole("button", { name: "已安全保存", exact: true }).click();
  await expect(
    page.getByText("已启用身份验证器，剩余 8 个恢复码。"),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "登录设备", exact: true }),
  ).toBeVisible();
  const cookie = (await context.cookies()).find(
    (cookie) => cookie.name === "ostool-admin-access",
  )!;
  expect(cookie).toMatchObject({
    httpOnly: true,
    sameSite: "Strict",
    path: "/",
  });
  await securityDb(async (client) => {
    const record = await client.query(
      "SELECT secret_ciphertext FROM mfa_credentials m JOIN users u ON u.id=m.user_id WHERE u.email=$1",
      [email],
    );
    expect(record.rows[0].secret_ciphertext).not.toContain(secret);
    await client.query(
      "UPDATE oauth_sessions SET step_up_until=now()-interval '1 minute' WHERE access_token_hash=$1",
      [createHash("sha256").update(cookie.value).digest("hex")],
    );
  });
  await page
    .getByRole("button", { name: "重新生成恢复码", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toContainText("验证当前身份");
  await page.getByLabel("当前密码", { exact: true }).fill(password);
  await page.getByLabel("验证码或恢复码", { exact: true }).fill(recovery[0]);
  await page.getByRole("button", { name: "验证身份", exact: true }).click();
  await expect(page.getByRole("dialog")).toContainText("保存恢复码");
  await expect(page.locator(".recovery-codes code")).toHaveCount(8);
  const replaced = await page.locator(".recovery-codes code").allTextContents();
  expect(replaced).toHaveLength(8);
  expect(replaced).not.toContain(recovery[0]);
  await page.getByRole("button", { name: "已安全保存", exact: true }).click();
  await page.getByRole("button", { name: "账号菜单", exact: true }).click();
  await page.getByRole("menuitem", { name: "退出登录", exact: true }).click();
  await page.getByLabel("邮箱", { exact: true }).fill(email);
  await page.getByLabel("密码", { exact: true }).fill(password);
  await solveCaptcha(page);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(
    page.getByLabel("验证码或恢复码", { exact: true }),
  ).toBeVisible();
  await page.getByLabel("验证码或恢复码", { exact: true }).fill(replaced[0]);
  await page.getByRole("button", { name: "验证并登录", exact: true }).click();
  await expect(page).toHaveURL(base + "/admin/overview");
  await page.goto(base + "/admin/account");
  await expect(
    page.getByText("已启用身份验证器，剩余 7 个恢复码。"),
  ).toBeVisible();
  expect(events.some((url) => url.includes("/auth/security/events"))).toBe(
    true,
  );
  expect(
    events.every(
      (url) => !url.includes("access_token") && !url.includes(cookie.value),
    ),
  ).toBe(true);
  const visible = await page.evaluate(() => ({
    cookie: document.cookie,
    storage: JSON.stringify(localStorage),
  }));
  expect(visible.cookie).not.toContain(cookie.value);
  expect(visible.storage).not.toContain(cookie.value);
  expect(visible.storage).not.toMatch(/access_token|refresh_token/);
  await page.screenshot({
    path: "/tmp/ostool-security-account.png",
    fullPage: true,
  });
  expect(errors).toEqual([]);
});

import { expect, test } from "playwright/test";
import { apiRegister, solveCaptcha } from "../fixtures/security";

test("website approves CLI device login only after terminal confirmation", async ({
  browser,
  request,
  playwright,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires the disposable public fixture");
  const email = `device-login-${Date.now()}@test.local`;
  expect(
    (
      await apiRegister(
        request,
        base!,
        email,
        "device-login-password-123",
        "CLI 用户",
      )
    ).ok(),
  ).toBeTruthy();
  const cli = await playwright.request.newContext();
  const deviceResponse = await cli.post(`${base}/oauth/device/code`, {
    form: { client_id: "ostool-cli", scope: "board:operate offline_access" },
  });
  expect(deviceResponse.status()).toBe(200);
  const device = await deviceResponse.json();
  const exchange = {
    grant_type: "urn:ietf:params:oauth:grant-type:device_code",
    client_id: "ostool-cli",
    device_code: device.device_code,
  };
  const pending = await cli.post(`${base}/oauth/token`, { form: exchange });
  expect((await pending.json()).error).toBe("authorization_pending");
  const context = await browser.newContext();
  try {
    const page = await context.newPage();
    await page.goto(device.verification_uri_complete);
    await expect(
      page.getByRole("heading", { name: "批准 CLI 登录" }),
    ).toBeVisible();
    await expect(page.locator(".auth-code code")).toHaveText(device.user_code);
    await page.getByLabel("邮箱", { exact: true }).fill(email);
    await page
      .getByLabel("密码", { exact: true })
      .fill("device-login-password-123");
    await solveCaptcha(page);
    const submit = page.getByRole("button", {
      name: "登录并批准",
      exact: true,
    });
    await expect(submit).toBeDisabled();
    await page
      .getByRole("checkbox", { name: "确认由我发起并核对终端验证码" })
      .check();
    await submit.click();
    await expect(
      page.getByText("设备登录已批准，可以返回 ostool CLI。"),
    ).toBeVisible();
    const grantResponse = await cli.post(`${base}/oauth/token`, {
      form: exchange,
    });
    expect(grantResponse.ok()).toBeTruthy();
    const grant = await grantResponse.json();
    expect(grant.access_token).toBeTruthy();
    expect(grant.refresh_token).toBeTruthy();
    const boardTypes = await cli.get(`${base}/api/v1/board-types`, {
      headers: { authorization: `Bearer ${grant.access_token}` },
    });
    expect(boardTypes.ok()).toBeTruthy();
    expect(await boardTypes.json()).toEqual([]);
    const rejected = await cli.get(`${base}/api/v1/account`, {
      headers: { authorization: `Bearer ${grant.access_token}` },
    });
    expect(rejected.status()).toBe(403);
  } finally {
    await context.close();
    await cli.dispose();
  }
});

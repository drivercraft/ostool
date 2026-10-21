import { expect, test } from "playwright/test";
import { apiLogin, apiRegister, solveCaptcha } from "../fixtures/security";

test("anonymous device cards receive updates and keep the selected board after login", async ({
  browser,
  request,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires the disposable public fixture");
  const suffix = Date.now();
  const { accessToken } = await apiLogin(
    request,
    base!,
    process.env.OSTOOL_E2E_ADMIN_EMAIL!,
    process.env.OSTOOL_E2E_ADMIN_PASSWORD!,
  );
  const headers = { authorization: `Bearer ${accessToken}` };
  const devices = [false, true].map((disabled) => ({
    id: `catalog-${suffix}-${disabled ? "disabled" : "enabled"}`,
    board_type: "catalog-test",
    tags: [`catalog-${suffix}`],
    disabled,
    serial: null,
    notes: "private operator notes",
    network_identity: null,
    boot: { kind: "pxe", notes: "private boot settings" },
    power_management: {
      kind: "custom",
      power_on_cmd: "true",
      power_off_cmd: "true",
    },
  }));
  for (const device of devices)
    expect(
      (
        await request.post(`${base}/api/v1/admin/boards`, {
          headers,
          data: device,
        })
      ).ok(),
    ).toBeTruthy();
  const email = `catalog-${suffix}@test.local`;
  expect(
    (
      await apiRegister(
        request,
        base!,
        email,
        "catalog-password-123",
        "设备目录用户",
      )
    ).ok(),
  ).toBeTruthy();
  const context = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
  });
  try {
    const page = await context.newPage();
    await page.goto(`${base}/devices`);
    await page.getByLabel("搜索设备").fill(`catalog-${suffix}`);
    const cards = page.locator(".device-directory-card");
    await expect(cards).toHaveCount(2);
    await expect(page.getByRole("table")).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "暂不可申请" }),
    ).toBeDisabled();
    const anonymous = await context.request.get(`${base}/api/v1/site/devices`);
    expect(anonymous.ok()).toBeTruthy();
    for (const item of await anonymous.json())
      expect(Object.keys(item).sort()).toEqual([
        "board_type",
        "disabled",
        "id",
        "tags",
      ]);
    // A public SSE push updates the mounted card while preserving its search.
    devices[1].disabled = false;
    expect(
      (
        await request.put(`${base}/api/v1/admin/boards/${devices[1].id}`, {
          headers,
          data: devices[1],
        })
      ).ok(),
    ).toBeTruthy();
    await expect(page.getByRole("link", { name: "登录后申请" })).toHaveCount(2);
    await expect(page.getByLabel("搜索设备")).toHaveValue(`catalog-${suffix}`);
    await cards
      .filter({ has: page.getByRole("heading", { name: devices[0].id }) })
      .getByRole("link", { name: "登录后申请" })
      .click();
    await page.getByLabel("邮箱", { exact: true }).fill(email);
    await page.getByLabel("密码", { exact: true }).fill("catalog-password-123");
    await solveCaptcha(page);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).toHaveURL(
      `${base}/account/requests/new?board=${devices[0].id}`,
    );
    await expect(
      page.getByRole("button", { name: "提交申请", exact: true }),
    ).toBeVisible();
    await expect(page.locator(".sidebar")).toHaveCount(0);
    await page.screenshot({ path: "/tmp/ostool-public-device-rental.png" });
    await page.goto(`${base}/devices`);
    await page.getByLabel("搜索设备").fill(`catalog-${suffix}`);
    await expect(
      page.getByRole("link", { name: "申请租赁", exact: true }),
    ).toHaveCount(2);
    await page.screenshot({ path: "/tmp/ostool-public-device-cards.png" });
    const tools = await page.locator(".device-directory-tools").boundingBox();
    const search = await page.getByLabel("搜索设备").boundingBox();
    expect(search!.x).toBeGreaterThan(tools!.x + tools!.width / 2);
    await page.setViewportSize({ width: 390, height: 844 });
    await expect(cards).toHaveCount(2);
    expect(
      await page.evaluate(
        () =>
          document.documentElement.scrollWidth <=
          document.documentElement.clientWidth,
      ),
    ).toBe(true);
    await page.screenshot({ path: "/tmp/ostool-public-device-mobile.png" });
  } finally {
    await context.close();
  }
});

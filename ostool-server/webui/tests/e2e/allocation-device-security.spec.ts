import { expect, test } from "playwright/test";
import { apiLogin, solveCaptcha } from "../fixtures/security";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

test("a member claims and returns an eligible board without gaining console access", async ({
  browser,
  request,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires disposable public fixture");
  const admin = await apiLogin(
    request,
    base!,
    process.env.OSTOOL_E2E_ADMIN_EMAIL!,
    process.env.OSTOOL_E2E_ADMIN_PASSWORD!,
  );
  const headers = { authorization: "Bearer " + admin.accessToken };
  const suffix = Date.now();
  const role = await request.post(base + "/api/v1/admin/roles", {
    headers,
    data: {
      code: "claim_" + suffix,
      enabled: true,
      name: "Claim " + suffix,
      description: "browser self allocation",
      permissions: ["board.instant"],
      allocation_policy: {
        max_duration_minutes: 30,
        max_concurrent_sessions: 1,
        min_available_minutes: 5,
        board_types: ["claim-browser"],
      },
    },
  });
  expect(role.status(), await role.text()).toBe(201);
  const roles = await (
    await request.get(base + "/api/v1/admin/roles", { headers })
  ).json();
  const roleData = roles.find(
    (item: { code: string }) => item.code === "claim_" + suffix,
  );
  expect(roleData).toBeTruthy();
  const username = "claim_" + suffix;
  const email = username + "@test.local";
  const password = "claim-browser-password-123";
  const member = await request.post(base + "/api/v1/admin/users", {
    headers,
    data: {
      username,
      email,
      password,
      display_name: "Claim member",
      role_id: roleData.id,
      disabled: false,
    },
  });
  expect(member.status(), await member.text()).toBe(201);
  const board = "claim-board-" + suffix;
  expect(
    (
      await request.post(base + "/api/v1/admin/boards", {
        headers,
        data: {
          id: board,
          board_type: "claim-browser",
          allow_instant_use: true,
          tags: [],
          disabled: false,
          serial: {
            key: { kind: "serial_number", value: "claim-console-" + suffix },
            baud_rate: 115200,
          },
          notes: null,
          network_identity: null,
          boot: { kind: "uboot", use_tftp: false },
          power_management: {
            kind: "custom",
            power_on_cmd: "true",
            power_off_cmd: "true",
          },
        },
      })
    ).status(),
  ).toBe(201);
  const context = await browser.newContext();
  try {
    const page = await context.newPage();
    await page.goto(base + "/login");
    await page.getByLabel(/^(邮箱|账号)$/, { exact: true }).fill(email);
    await page.getByLabel("密码", { exact: true }).fill(password);
    await solveCaptcha(page);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).toHaveURL(base + "/account");
    await page.getByRole("link", { name: "即时领取开发板" }).click();
    await page.getByLabel("开发板", { exact: true }).selectOption(board);
    await page.getByRole("button", { name: "确认领取", exact: true }).click();
    await expect(page).toHaveURL(base + "/account/rentals");
    const row = page
      .getByRole("row")
      .filter({ has: page.getByText(board, { exact: true }) });
    await expect(row).toContainText("使用期内");
    expect(
      (await page.request.get(base + "/api/v1/admin/boards")).status(),
    ).toBe(401);
    await row.getByRole("button", { name: "归还设备" }).click();
    await page.getByRole("button", { name: "确认", exact: true }).click();
    await expect(row).toContainText("已取消");
  } finally {
    await context.close();
  }
});

test("the board editor keeps credentials write-only and preserves a draft across authenticated updates", async ({
  browser,
  request,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires disposable public fixture");
  const admin = await apiLogin(
    request,
    base!,
    process.env.OSTOOL_E2E_ADMIN_EMAIL!,
    process.env.OSTOOL_E2E_ADMIN_PASSWORD!,
  );
  const headers = { authorization: "Bearer " + admin.accessToken };
  const id = "tls-board-" + Date.now();
  expect(
    (
      await request.post(base + "/api/v1/admin/boards", {
        headers,
        data: {
          id,
          board_type: "tls-browser",
          tags: [],
          disabled: false,
          serial: null,
          notes: null,
          network_identity: { mac_address: "02:00:00:11:09:42" },
          boot: { kind: "httpboot" },
          power_management: {
            kind: "custom",
            power_on_cmd: "true",
            power_off_cmd: "true",
          },
        },
      })
    ).status(),
  ).toBe(201);
  const context = await browser.newContext();
  try {
    const page = await context.newPage();
    await page.goto(
      base +
        "/admin/login?redirect=" +
        encodeURIComponent("/admin/boards/" + id),
    );
    await page
      .getByLabel(/^(邮箱|账号)$/, { exact: true })
      .fill(process.env.OSTOOL_E2E_ADMIN_EMAIL!);
    await page
      .getByLabel("密码", { exact: true })
      .fill(process.env.OSTOOL_E2E_ADMIN_PASSWORD!);
    await solveCaptcha(page);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).toHaveURL(base + "/admin/boards/" + id, {
      timeout: 15000,
    });
    await expect(
      page.getByRole("checkbox", { name: "禁用开发板" }),
    ).toBeChecked();
    const endpoint = page.getByLabel("设备 HTTPS 地址");
    await expect(endpoint).toBeVisible();
    await endpoint.fill("https://127.0.0.1:1");
    const token = "separate-device-credential-for-browser-fixture";
    await page.getByLabel("设备独立凭据", { exact: true }).fill(token);
    const ca = await readFile(
      resolve("../tests/fixtures/device-tls/ca.pem"),
      "utf8",
    );
    await page.getByLabel("设备 CA 证书（可选）").fill(ca);
    await page
      .getByRole("button", { name: "保存设备认证", exact: true })
      .click();
    await expect(page.getByLabel("设备独立凭据", { exact: true })).toHaveValue(
      "",
    );
    await expect(
      page.getByText("服务器认证配置已更新，输入已保留。"),
    ).toHaveCount(0);
    const view = await request.get(
      base + "/api/v1/admin/boards/" + id + "/device-connection",
      { headers },
    );
    expect(view.status()).toBe(200);
    expect(await view.text()).not.toContain(token);
    await endpoint.fill("https://draft.example.com");
    expect(
      (
        await request.put(
          base + "/api/v1/admin/boards/" + id + "/device-connection",
          { headers, data: { endpoint: "https://127.0.0.1:2", ca_pem: ca } },
        )
      ).status(),
    ).toBe(200);
    await expect(
      page.getByText("服务器认证配置已更新，输入已保留。"),
    ).toBeVisible();
    await expect(endpoint).toHaveValue("https://draft.example.com");
    await page
      .getByRole("button", { name: "载入最新配置", exact: true })
      .click();
    await expect(endpoint).toHaveValue("https://127.0.0.1:2");
  } finally {
    await request.delete(
      base + "/api/v1/admin/boards/" + id + "/device-connection",
      { headers },
    );
    await context.close();
  }
});

import {
  solveCaptcha,
  securityDb,
  apiLogin,
  apiRegister,
} from "../fixtures/security";
import { createHash } from "node:crypto";
import { expect, test } from "playwright/test";

// Optional second server with PostgreSQL; the ordinary fixture stays isolated.
test("registration, member/admin access, user/role events and settings navigation", async ({
  page,
  request,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires a disposable PostgreSQL-backed server");
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${base}/login`);
  await page
    .getByRole("link", { name: "还没有账号？免费注册", exact: true })
    .first()
    .click();
  await expect(
    page.getByRole("heading", { name: "创建你的账号" }),
  ).toBeVisible();
  const email = `browser-${Date.now()}@test.local`;
  await page.getByLabel("用户名", { exact: true }).fill(email.split("@")[0]);
  await page.getByLabel(/^(邮箱|账号)$/, { exact: true }).fill(email);
  await page.getByLabel("显示名称", { exact: true }).fill("浏览器注册用户");
  await page.getByLabel("密码", { exact: true }).fill("browser-password-123");
  await page
    .getByLabel("确认密码", { exact: true })
    .fill("browser-password-123");
  await solveCaptcha(page);
  await page
    .getByRole("checkbox", { name: "我已阅读并同意用户协议和隐私政策" })
    .check();
  await page.getByRole("button", { name: "创建账号", exact: true }).click();
  await expect(page).toHaveURL(`${base}/account`);
  await expect(page.getByRole("heading", { name: "当前账号" })).toBeVisible();
  await page.goto(`${base}/admin/users`);
  await expect(
    page.getByRole("heading", { name: "管理后台登录" }),
  ).toBeVisible();
  await page.getByLabel(/^(邮箱|账号)$/, { exact: true }).fill(email);
  await page.getByLabel("密码", { exact: true }).fill("browser-password-123");
  await solveCaptcha(page);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(
    page.getByText("用户名、邮箱或密码不正确", { exact: true }),
  ).toBeVisible();
  await page.goto(`${base}/admin/login?redirect=/admin/overview`);
  await page
    .getByLabel(/^(邮箱|账号)$/, { exact: true })
    .fill(process.env.OSTOOL_E2E_ADMIN_EMAIL ?? "admin@example.com");
  await page
    .getByLabel("密码", { exact: true })
    .fill(process.env.OSTOOL_E2E_ADMIN_PASSWORD ?? "admin-password-123");
  await solveCaptcha(page);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(page).toHaveURL(`${base}/admin/overview`);
  await page.getByRole("link", { name: "用户列表", exact: true }).click();
  await expect(page.getByText(email, { exact: true })).toBeVisible();
  await page.screenshot({
    path: "/tmp/ostool-users-verified.png",
    fullPage: true,
  });
  await page.getByRole("link", { name: "用户角色", exact: true }).click();
  await page.getByRole("button", { name: "创建角色", exact: true }).click();
  const role = `浏览器测试角色 ${Date.now()}`;
  await page
    .getByLabel("角色编码", { exact: true })
    .fill(`browser_role_${Date.now()}`);
  await page.getByLabel("角色名称", { exact: true }).fill(role);
  await page.getByRole("button", { name: "保存角色", exact: true }).click();
  await expect(page.getByText(role, { exact: true })).toBeVisible();
  const token = (await page.context().cookies()).find(
    (cookie) => cookie.name === "ostool-admin-access",
  )!.value;
  const headers = { authorization: `Bearer ${token}` };
  const boardId = `rental-board-${Date.now()}`;
  expect(
    (
      await request.post(`${base}/api/v1/admin/boards`, {
        headers,
        data: {
          id: boardId,
          board_type: "test-rental",
          tags: [],
          disabled: false,
          serial: null,
          notes: null,
          network_identity: null,
          boot: { kind: "pxe", notes: null },
          power_management: {
            kind: "custom",
            power_on_cmd: "true",
            power_off_cmd: "true",
          },
        },
      })
    ).ok(),
  ).toBeTruthy();
  await page.getByRole("link", { name: "租赁列表", exact: true }).click();
  await page.getByRole("link", { name: "新增租赁", exact: true }).click();
  await expect(page).toHaveURL(`${base}/admin/rentals/new`);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("region", { name: "租赁日历" })).toBeVisible();
  const userSelect = page.getByLabel("使用用户", { exact: true });
  const userValue = await userSelect
    .locator("option")
    .filter({ hasText: email })
    .getAttribute("value");
  await userSelect.selectOption(userValue!);
  await page.getByLabel("指定开发板", { exact: true }).selectOption(boardId);
  await page.getByRole("button", { name: "保存租赁", exact: true }).click();
  const row = page
    .getByRole("row")
    .filter({ has: page.getByText(boardId, { exact: true }) });
  await expect(row).toBeVisible();
  await expect(row.getByText("使用期内", { exact: true })).toBeVisible();
  await page.screenshot({
    path: "/tmp/ostool-rentals-verified.png",
    fullPage: true,
  });
  await row.getByRole("button", { name: "取消租赁", exact: true }).click();
  await page.getByRole("button", { name: "确认", exact: true }).click();
  await expect(row.getByText("已取消", { exact: true })).toBeVisible();
  await page.getByLabel("租赁状态", { exact: true }).selectOption("active");
  await expect(row).toHaveCount(0);
  await page.getByRole("link", { name: "系统设置", exact: true }).click();
  await page.getByRole("tab", { name: "网络与上传", exact: true }).click();
  await expect(page).toHaveURL(/tab=server/);
  await page.reload();
  await expect(page.getByRole("tab", { name: "网络与上传" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: "/tmp/ostool-settings-mobile.png",
    fullPage: true,
  });
  await page.goto(`${base}/register`);
  await page.screenshot({
    path: "/tmp/ostool-registration-mobile.png",
    fullPage: true,
  });
  expect(errors).toEqual([]);
});

test("temporary refresh failures preserve the administrator login and unsaved device draft", async ({
  page,
  context,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires disposable public PostgreSQL and Redis fixtures");
  test.setTimeout(60000);
  await page.goto(base + "/admin/login?redirect=/admin/boards/new");
  await page
    .getByLabel(/^(邮箱|账号)$/, { exact: true })
    .fill(process.env.OSTOOL_E2E_ADMIN_EMAIL!);
  await page
    .getByLabel("密码", { exact: true })
    .fill(process.env.OSTOOL_E2E_ADMIN_PASSWORD!);
  await solveCaptcha(page);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(page).toHaveURL(base + "/admin/boards/new");
  await page
    .getByLabel("备注", { exact: true })
    .fill("keep this draft across a restart");
  const original = (await context.cookies()).find(
    (cookie) => cookie.name === "ostool-admin-access",
  )!;
  let attempts = 0;
  await page.route("**/api/v1/auth/refresh", async (route) => {
    attempts++;
    await route.fulfill({
      status: 503,
      contentType: "application/json",
      body: JSON.stringify({
        code: "service_unavailable",
        message: "temporarily restarting",
      }),
    });
  });
  try {
    await securityDb((client) =>
      client.query(
        "UPDATE oauth_sessions SET access_expires_at=now()-interval '1 minute' WHERE access_token_hash=$1",
        [createHash("sha256").update(original.value).digest("hex")],
      ),
    );
    await expect.poll(() => attempts, { timeout: 25000 }).toBeGreaterThan(0);
    await expect(page).toHaveURL(base + "/admin/boards/new");
    await expect(page.getByLabel("备注", { exact: true })).toHaveValue(
      "keep this draft across a restart",
    );
  } finally {
    await page.unroute("**/api/v1/auth/refresh");
  }
  await expect
    .poll(
      async () =>
        (await context.cookies()).find(
          (cookie) => cookie.name === "ostool-admin-access",
        )?.value !== original.value,
      { timeout: 20000 },
    )
    .toBe(true);
  await expect(page).toHaveURL(base + "/admin/boards/new");
  await expect(page.getByLabel("备注", { exact: true })).toHaveValue(
    "keep this draft across a restart",
  );
});

import { solveCaptcha, apiLogin, apiRegister } from "../fixtures/security";
import { expect, test } from "playwright/test";

test("public member profile, rental request approval, theme and password lifecycle", async ({
  browser,
  request,
}) => {
  test.setTimeout(60000);
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires a disposable PostgreSQL-backed server");
  const adminEmail = process.env.OSTOOL_E2E_ADMIN_EMAIL ?? "admin@example.com";
  const adminPassword =
    process.env.OSTOOL_E2E_ADMIN_PASSWORD ?? "admin-password-123";
  const { accessToken: adminToken } = await apiLogin(
    request,
    base!,
    adminEmail,
    adminPassword,
  );
  const headers = { authorization: `Bearer ${adminToken}` };
  const suffix = Date.now();
  const board = `member-board-${suffix}`;
  expect(
    (
      await request.post(`${base}/api/v1/admin/boards`, {
        headers,
        data: {
          id: board,
          board_type: "member-test",
          tags: ["browser"],
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
  const email = `workspace-${suffix}@test.local`;
  expect(
    (
      await apiRegister(
        request,
        base!,
        email,
        "member-password-123",
        "浏览器租赁成员",
      )
    ).ok(),
  ).toBeTruthy();
  const memberContext = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
  });
  const adminContext = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
  });
  try {
    const page = await memberContext.newPage(),
      admin = await adminContext.newPage();
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    admin.on("pageerror", (e) => errors.push(e.message));
    await page.goto(`${base}/`);
    const logo = await page.locator(".site-header .site-logo").boundingBox();
    const nav = await page
      .getByRole("navigation", { name: "网站导航" })
      .boundingBox();
    expect(nav!.x - logo!.x - logo!.width).toBeLessThan(80);
    await page.getByRole("button", { name: "切换网站主题" }).click();
    await page
      .getByRole("menuitemradio", { name: "黑暗", exact: true })
      .click();
    await expect(page.locator("html")).toHaveClass(/dark/);
    await page.reload();
    await expect(page.locator("html")).toHaveClass(/dark/);
    await page.getByRole("button", { name: "切换网站主题" }).click();
    await page
      .getByRole("menuitemradio", { name: "明亮", exact: true })
      .click();
    await page.goto(`${base}/login`);
    await page.getByLabel(/^(邮箱|账号)$/, { exact: true }).fill(email);
    await page.getByLabel("密码", { exact: true }).fill("member-password-123");
    await solveCaptcha(page);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).toHaveURL(`${base}/account`);
    await expect(page.getByRole("heading", { name: "当前账号" })).toBeVisible();
    await expect(page.locator(".sidebar")).toHaveCount(0);
    await page.getByRole("button", { name: "用户菜单" }).click();
    await expect(
      page.getByRole("menuitem", { name: "管理后台", exact: true }),
    ).toHaveCount(0);
    await page.getByRole("menuitem", { name: "基本信息", exact: true }).click();
    await page.getByLabel("显示名称", { exact: true }).fill("已更新的租赁成员");
    await page.getByRole("button", { name: "保存资料" }).click();
    await expect(
      page.locator('.site-actions [aria-label="用户菜单"]'),
    ).toHaveAttribute("title", "已更新的租赁成员");
    await page.reload();
    await expect(page.getByLabel("显示名称", { exact: true })).toHaveValue(
      "已更新的租赁成员",
    );
    await page.goto(`${base}/devices`);
    await page.getByLabel("搜索设备").fill(board);
    await page.getByRole("link", { name: "申请租赁", exact: true }).click();
    await expect(page).toHaveURL(base + "/account/requests/new?board=" + board);
    const editor = page.locator(".rental-request-panel");
    await expect(
      editor.getByRole("region", { name: "租赁日历" }),
    ).toBeVisible();
    await expect(
      editor
        .locator(".panel-footer")
        .getByRole("button", { name: "提交申请", exact: true }),
    ).toBeVisible();
    const fieldSizes = await editor
      .locator(".rental-editor-form input,.rental-editor-form select")
      .evaluateAll((elements) =>
        elements.map((el) => ({
          height: el.getBoundingClientRect().height,
          top: el.getBoundingClientRect().top,
        })),
      );
    expect(fieldSizes.every((field) => field.height === 42)).toBe(true);
    expect(fieldSizes[0].top).toBe(fieldSizes[1].top);
    expect(fieldSizes[2].top).toBe(fieldSizes[3].top);
    await expect(page.getByLabel("申请开发板", { exact: true })).toHaveValue(
      board,
    );
    await page.getByLabel("申请说明").fill("浏览器端到端租赁申请");
    await page.screenshot({
      path: "/tmp/ostool-member-rental-editor.png",
      fullPage: true,
    });
    await page.getByRole("button", { name: "提交申请" }).click();
    const memberRow = page
      .getByRole("row")
      .filter({ has: page.getByRole("cell", { name: board, exact: false }) });
    await expect(memberRow.getByText("待审核", { exact: true })).toBeVisible();
    await page.screenshot({
      path: "/tmp/ostool-member-requests.png",
      fullPage: true,
    });
    await admin.goto(`${base}/admin/rental-requests`);
    await admin.getByLabel(/^(邮箱|账号)$/, { exact: true }).fill(adminEmail);
    await admin.getByLabel("密码", { exact: true }).fill(adminPassword);
    await solveCaptcha(admin);
    await admin.getByRole("button", { name: "登录", exact: true }).click();
    await expect(admin).toHaveURL(`${base}/admin/rental-requests`);
    await admin.getByLabel("搜索申请").fill(email);
    const adminRow = admin
      .getByRole("row")
      .filter({ has: admin.getByRole("cell", { name: email, exact: true }) });
    await adminRow.getByRole("button", { name: "批准", exact: true }).click();
    await admin.getByLabel("审核意见").fill("按时归还开发板");
    await admin.getByRole("button", { name: "确认审核", exact: true }).click();
    await expect(memberRow.getByText("已通过", { exact: true })).toBeVisible();
    await expect(
      memberRow.getByText("按时归还开发板", { exact: true }),
    ).toBeVisible();
    await page
      .getByRole("navigation", { name: "个人中心导航" })
      .getByRole("link", { name: "我的租赁", exact: true })
      .click();
    await expect(
      page.getByRole("cell", { name: board, exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: "/tmp/ostool-member-rentals.png",
      fullPage: true,
    });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.getByRole("button", { name: "切换网站主题" }).click();
    await page
      .getByRole("menuitemradio", { name: "黑暗", exact: true })
      .click();
    await page.screenshot({
      path: "/tmp/ostool-member-mobile-dark.png",
      fullPage: true,
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    await page.goto(`${base}/account/security`);
    await expect(
      page.getByRole("heading", { name: "账号安全", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "双重验证", exact: true }),
    ).toBeVisible();
    await expect(page.getByLabel("当前密码", { exact: true })).toHaveCount(0);
    await page.goto(`${base}/account/password`);
    await expect(
      page.getByRole("heading", { name: "修改密码", exact: true }),
    ).toBeVisible();
    await page
      .getByLabel("当前密码", { exact: true })
      .fill("member-password-123");
    await page
      .getByLabel("新密码", { exact: true })
      .fill("changed-member-password-123");
    await page
      .getByLabel("确认新密码", { exact: true })
      .fill("changed-member-password-123");
    await page.getByRole("button", { name: "更新密码" }).click();
    await expect(page).toHaveURL(`${base}/login?password_changed=1`);
    await page.getByLabel(/^(邮箱|账号)$/, { exact: true }).fill(email);
    await page
      .getByLabel("密码", { exact: true })
      .fill("changed-member-password-123");
    await solveCaptcha(page);
    await page.getByRole("button", { name: "登录", exact: true }).click();
    await expect(page).toHaveURL(`${base}/account`);
    expect(errors).toEqual([]);
  } finally {
    await memberContext.close();
    await adminContext.close();
  }
});

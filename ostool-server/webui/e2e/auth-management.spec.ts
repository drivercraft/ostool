import { expect, test } from "playwright/test";

// Optional second server with PostgreSQL; the ordinary fixture stays isolated.
test("registration, member/admin access, user/role events and settings navigation", async ({
  page,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires a disposable PostgreSQL-backed server");
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${base}/login`);
  await page
    .getByRole("link", { name: "免费注册", exact: true })
    .first()
    .click();
  await expect(
    page.getByRole("heading", { name: "创建你的账号" }),
  ).toBeVisible();
  const email = `browser-${Date.now()}@test.local`;
  await page.getByLabel("邮箱", { exact: true }).fill(email);
  await page.getByLabel("显示名称", { exact: true }).fill("浏览器注册用户");
  await page.getByLabel("密码", { exact: true }).fill("browser-password-123");
  await page
    .getByLabel("确认密码", { exact: true })
    .fill("browser-password-123");
  await page.getByRole("button", { name: "创建账号", exact: true }).click();
  await expect(page).toHaveURL(`${base}/account`);
  await expect(
    page.getByRole("heading", { name: "账号与访问令牌" }),
  ).toBeVisible();
  await page.goto(`${base}/admin/users`);
  await expect(
    page.getByText("你的账号没有管理后台权限，请联系管理员分配角色。"),
  ).toBeVisible();
  await page.goto(`${base}/login`);
  await page
    .getByLabel("邮箱", { exact: true })
    .fill(process.env.OSTOOL_E2E_ADMIN_EMAIL ?? "admin@example.com");
  await page
    .getByLabel("密码", { exact: true })
    .fill(process.env.OSTOOL_E2E_ADMIN_PASSWORD ?? "admin-password-123");
  await page.getByRole("button", { name: "登录控制台", exact: true }).click();
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
  await page.getByLabel("角色名称", { exact: true }).fill(role);
  await page.getByRole("button", { name: "保存角色", exact: true }).click();
  await expect(page.getByText(role, { exact: true })).toBeVisible();
  await page.getByRole("link", { name: "系统设置", exact: true }).click();
  await page.getByRole("tab", { name: "数据存储", exact: true }).click();
  await expect(page).toHaveURL(/tab=storage/);
  await page.reload();
  await expect(page.getByRole("tab", { name: "数据存储" })).toHaveAttribute(
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

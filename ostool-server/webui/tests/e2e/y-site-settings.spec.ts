import { expect, test } from "playwright/test";
import { solveCaptcha, securityDb } from "../fixtures/security";
test("site settings publish through SSE, legal pages and explicit versioned signup consent", async ({
  page,
  context,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires disposable public PostgreSQL and Redis fixtures");
  test.setTimeout(60000);
  const website = await context.newPage();
  await website.goto(base + "/login");
  await expect(
    website.getByRole("link", { name: "用户协议", exact: true }).first(),
  ).toBeVisible();
  await expect(
    website.getByRole("link", { name: "隐私政策", exact: true }).first(),
  ).toBeVisible();
  await page.goto(base + "/admin/login?redirect=/admin/settings");
  await page
    .getByLabel("邮箱", { exact: true })
    .fill(process.env.OSTOOL_E2E_ADMIN_EMAIL!);
  await page
    .getByLabel("密码", { exact: true })
    .fill(process.env.OSTOOL_E2E_ADMIN_PASSWORD!);
  await solveCaptcha(page);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(page).toHaveURL(base + "/admin/settings");
  await page.getByRole("tab", { name: "站点信息", exact: true }).click();
  await page.getByLabel("站点名称", { exact: true }).fill("开发板协作测试");
  await page.getByLabel("站点简称", { exact: true }).fill("协作测试");
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(
    website.locator(".site-header .site-logo-copy strong"),
  ).toHaveText("协作测试");
  await expect(website).toHaveTitle(/开发板协作测试/);
  await page.getByRole("tab", { name: "协议与隐私", exact: true }).click();
  const text = "版本化用户协议：只能在授权时段使用开发板。";
  await page.getByLabel("用户协议正文", { exact: true }).fill(text);
  await page.getByRole("button", { name: "发布协议", exact: true }).click();
  await expect(
    page.getByRole("cell", { name: "v2", exact: true }),
  ).toBeVisible();
  await website.goto(base + "/legal/terms");
  await expect(website.getByRole("article")).toHaveText(text);
  await website.reload();
  await expect(website.getByRole("article")).toHaveText(text);
  await website.goto(base + "/register");
  await website
    .getByLabel("邮箱", { exact: true })
    .fill("legal-" + Date.now() + "@test.local");
  await website.getByLabel("显示名称", { exact: true }).fill("协议测试成员");
  await website
    .getByLabel("密码", { exact: true })
    .fill("legal-browser-password-123");
  await website
    .getByLabel("确认密码", { exact: true })
    .fill("legal-browser-password-123");
  await solveCaptcha(website);
  await expect(
    website.getByRole("button", { name: "创建账号", exact: true }),
  ).toBeDisabled();
  await website
    .getByRole("checkbox", { name: "我已阅读并同意用户协议和隐私政策" })
    .check();
  await website.getByRole("button", { name: "创建账号", exact: true }).click();
  await expect(website).toHaveURL(base + "/account");
  await securityDb(async (client) => {
    const rows = await client.query(
      "SELECT kind,version FROM user_legal_acceptances a JOIN users u ON u.id=a.user_id WHERE u.display_name='协议测试成员' ORDER BY kind",
    );
    expect(rows.rows).toEqual([
      { kind: "privacy", version: "1" },
      { kind: "terms", version: "2" },
    ]);
  });
  await page.screenshot({
    path: "/tmp/ostool-site-legal-settings.png",
    fullPage: true,
  });
  await website.close();
});

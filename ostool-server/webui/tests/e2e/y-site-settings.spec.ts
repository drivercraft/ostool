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
    website.locator(".auth-aside .site-logo-copy strong"),
  ).toHaveText("协作测试");
  await expect(website).toHaveTitle(/开发板协作测试/);
  await page.getByRole("tab", { name: "用户协议", exact: true }).click();
  const text = "版本化用户协议：只能在授权时段使用开发板。";
  await page.getByLabel("用户协议正文", { exact: true }).fill(text);
  await page.getByRole("button", { name: "预览", exact: true }).click();
  await expect(
    page.getByRole("article", { name: "用户协议预览" }),
  ).toContainText(text);
  await page.getByRole("button", { name: "全屏", exact: true }).click();
  await expect(page.locator(".legal-fullscreen")).toBeVisible();
  const editor = page.getByLabel("用户协议正文", { exact: true });
  expect((await editor.boundingBox())!.height).toBeGreaterThan(300);
  await page.screenshot({ path: "/tmp/ostool-legal-editor-fullscreen.png" });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await expect(editor).toHaveValue(text);
  await page.screenshot({ path: "/tmp/ostool-legal-editor-mobile.png" });
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.keyboard.press("Escape");
  await expect(page.locator(".legal-fullscreen")).toHaveCount(0);
  await page.getByRole("tab", { name: "隐私政策", exact: true }).click();
  await page
    .getByLabel("隐私政策正文", { exact: true })
    .fill("尚未发布的隐私草稿");
  await page.getByRole("tab", { name: "用户协议", exact: true }).click();
  await expect(editor).toHaveValue(text);
  await page.getByRole("button", { name: "发布协议", exact: true }).click();
  await page.getByRole("button", { name: "发布历史", exact: true }).click();
  await expect(
    page.getByRole("cell", { name: "v2", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "查看版本 v1", exact: true }).click();
  await expect(
    page.getByRole("article", { name: "历史版本正文" }),
  ).toContainText("v1");
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: "隐私政策", exact: true }).click();
  await expect(page.getByLabel("隐私政策正文", { exact: true })).toHaveValue(
    "尚未发布的隐私草稿",
  );
  await expect(
    page.getByRole("button", { name: "发布协议", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "取消", exact: true }).click();
  await page.getByRole("tab", { name: "联系与备案", exact: true }).click();
  await page.getByLabel("服务时间", { exact: true }).fill("工作日 10:00–19:00");
  await page
    .getByLabel("隐私事务邮箱", { exact: true })
    .fill("privacy@test.local");
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "保存配置", exact: true }),
  ).toBeDisabled();
  await page.getByRole("tab", { name: "登录与部署", exact: true }).click();
  const registration = page.getByRole("checkbox", {
    name: "允许站点用户注册",
    exact: true,
  });
  await registration.uncheck();
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(
    website.getByRole("button", { name: "创建账号", exact: true }),
  ).toHaveCount(0);
  await website.goto(base + "/register");
  await expect(
    website.getByText("当前站点未开放注册，请联系管理员。", { exact: true }),
  ).toBeVisible();
  await registration.check();
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(
    website.getByText("当前站点未开放注册，请联系管理员。", { exact: true }),
  ).toHaveCount(0);
  await page.getByRole("tab", { name: "邮件服务", exact: true }).click();
  await expect(page.getByLabel("SMTP 主机", { exact: true })).toHaveAttribute(
    "placeholder",
    "smtp.example.com",
  );
  await expect(page.getByLabel("SMTP 密码", { exact: true })).toBeDisabled();
  await website.goto(base + "/legal/privacy");
  await expect(
    website.getByRole("link", { name: "privacy@test.local", exact: true }),
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

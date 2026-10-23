import { expect, test } from "playwright/test";
import { apiLogin, solveCaptcha } from "../fixtures/security";

const embeddedBase = () =>
  process.env.OSTOOL_E2E_AUTH_BASE_URL ?? "http://127.0.0.1:4175";

test("public shell renders before slow configuration, with a full hero and a footer at the bottom", async ({
  page,
}) => {
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/api/v1/auth/config", async (route) => {
    await pending;
    await route.continue();
  });
  try {
    await page.setViewportSize({ width: 1440, height: 960 });
    await page.goto(embeddedBase() + "/", { waitUntil: "domcontentloaded" });
    await expect(
      page.getByRole("navigation", { name: "网站导航" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: /连接每一块开发板/ }),
    ).toBeVisible();
    const header = await page.locator(".site-header").boundingBox();
    const hero = await page.locator(".public-hero-section").boundingBox();
    expect(Math.abs(hero!.height + header!.height - 960)).toBeLessThan(3);
    await expect(page.locator(".startup-shell")).toHaveCount(0);
    await page.screenshot({ path: "/tmp/ostool-home-desktop.png" });
  } finally {
    release();
    await page.unrouteAll({ behavior: "wait" });
  }
  await page.goto(embeddedBase() + "/about");
  const footer = await page.locator(".site-footer").boundingBox();
  expect(footer!.y + footer!.height).toBeGreaterThanOrEqual(958);
});

test("documentation uses three columns, changes articles, and remains responsive", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto(embeddedBase() + "/docs");
  await expect(
    page.getByRole("navigation", { name: "文档菜单" }),
  ).toBeVisible();
  await expect(
    page.getByRole("navigation", { name: "本页目录" }),
  ).toBeVisible();
  const left = await page.locator(".docs-sidebar").boundingBox();
  const article = await page.locator(".docs-article").boundingBox();
  const right = await page.locator(".docs-toc").boundingBox();
  expect(left!.x + left!.width).toBeLessThan(article!.x);
  expect(article!.x + article!.width).toBeLessThan(right!.x);
  await page.screenshot({ path: "/tmp/ostool-docs-desktop.png" });
  await page.getByRole("button", { name: "命令行工具", exact: true }).click();
  await expect(page.locator(".docs-article h1")).toHaveText("命令行工具");
  await expect(page).toHaveURL(/page=cli/);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await expect(page.locator(".docs-article")).toBeVisible();
  await page.screenshot({
    path: "/tmp/ostool-docs-mobile.png",
    fullPage: true,
  });
});

test("username signup and login are independent of email", async ({ page }) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires disposable public fixtures");
  const username = "engineer_" + Date.now();
  await page.goto(base + "/register");
  await page.getByLabel("用户名", { exact: true }).fill(username);
  await page.getByLabel("邮箱", { exact: true }).fill(username + "@test.local");
  await page
    .getByLabel("密码", { exact: true })
    .fill("username-browser-password-123");
  await page
    .getByLabel("确认密码", { exact: true })
    .fill("username-browser-password-123");
  await solveCaptcha(page);
  await page
    .getByRole("checkbox", { name: "我已阅读并同意用户协议和隐私政策" })
    .check();
  await page.getByRole("button", { name: "创建账号", exact: true }).click();
  await expect(page).toHaveURL(base + "/account");
  await page.goto(base + "/account/profile");
  await expect(page.getByLabel("用户名", { exact: true })).toHaveValue(
    username,
  );
  await page
    .getByRole("button", { name: "用户菜单", exact: true })
    .first()
    .click();
  await page.getByRole("menuitem", { name: "退出登录", exact: true }).click();
  await expect(page).toHaveURL(base + "/");
  await page.goto(base + "/login");
  await page
    .getByLabel("账号", { exact: true })
    .fill(username.toUpperCase());
  await page
    .getByLabel("密码", { exact: true })
    .fill("username-browser-password-123");
  await solveCaptcha(page);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(page).toHaveURL(base + "/account");
});

test("administrator shell stays full sized while identity loads and subscribes only after authorization", async ({
  page,
  context,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires disposable public fixtures");
  await apiLogin(
    context.request,
    base!,
    process.env.OSTOOL_E2E_ADMIN_EMAIL!,
    process.env.OSTOOL_E2E_ADMIN_PASSWORD!,
  );
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  let subscriptions = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/v1/admin/events")) subscriptions++;
  });
  await page.route("**/api/v1/auth/me", async (route) => {
    await pending;
    await route.continue();
  });
  await page.setViewportSize({ width: 1440, height: 960 });
  try {
    await page.goto(base + "/admin/boards", { waitUntil: "domcontentloaded" });
    await expect(page.locator(".sidebar")).toBeVisible();
    await expect(page.locator(".admin-workspace")).toHaveAttribute(
      "aria-busy",
      "true",
    );
    const bounds = await page.locator(".app-shell").boundingBox();
    expect(bounds!.height).toBeGreaterThanOrEqual(960);
    expect(subscriptions).toBe(0);
  } finally {
    release();
    await page.unrouteAll({ behavior: "wait" });
  }
  await expect(page.locator(".admin-workspace")).toHaveAttribute(
    "aria-busy",
    "false",
  );
  await expect(
    page.getByRole("toolbar", { name: "列表快捷工具栏" }),
  ).toBeVisible();
  expect(subscriptions).toBeGreaterThan(0);
});

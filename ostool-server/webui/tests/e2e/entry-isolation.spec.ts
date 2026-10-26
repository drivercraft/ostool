import { solveCaptcha, apiLogin, apiRegister } from "../fixtures/security";
import { expect, test } from "playwright/test";

test("server and dev host serve independent website and admin entry documents", async ({
  page,
  request,
}) => {
  for (const base of ["http://127.0.0.1:4175", "http://127.0.0.1:4174"]) {
    const site = await request.get(`${base}/`, {
      headers: { accept: "text/html" },
    });
    const admin = await request.get(`${base}/admin/login`, {
      headers: { accept: "text/html" },
    });
    expect(site.ok()).toBeTruthy();
    expect(admin.ok()).toBeTruthy();
    expect(await site.text()).toContain("开发板协作平台");
    expect(await site.text()).not.toContain("管理后台");
    expect(await admin.text()).toContain("管理后台");
    expect(await admin.text()).not.toContain("开发板协作平台");
    await page.goto(`${base}/`);
    await expect(
      page.getByRole("navigation", { name: "网站导航" }),
    ).toBeVisible();
    await page.reload();
    await expect(
      page.getByRole("navigation", { name: "网站导航" }),
    ).toBeVisible();
    await expect(page.locator(".app-shell, #admin-sidebar")).toHaveCount(0);
    await expect(page).not.toHaveTitle(/管理台|管理后台/);
  }
});

test("website and admin login/logout work independently in the same browser", async ({
  page,
  context,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires a disposable PostgreSQL public server");
  test.setTimeout(60_000);
  const email = process.env.OSTOOL_E2E_ADMIN_EMAIL ?? "admin@example.com";
  const password =
    process.env.OSTOOL_E2E_ADMIN_PASSWORD ?? "admin-password-123";
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${base}/login?redirect=/admin/overview`);
  await page.getByLabel(/^(邮箱|账号)$/, { exact: true }).fill(email);
  await page.getByLabel("密码", { exact: true }).fill(password);
  await solveCaptcha(page);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(page).toHaveURL(`${base}/account`);
  const websiteToken = (await context.cookies()).find(
    (cookie) => cookie.name === "ostool-website-access",
  )!.value;
  expect(
    (await context.cookies()).find(
      (cookie) => cookie.name === "ostool-website-access",
    ),
  ).toMatchObject({ httpOnly: true, sameSite: "Strict" });
  expect(websiteToken).toBeTruthy();
  const admin = await context.newPage();
  admin.on("pageerror", (error) => errors.push(error.message));
  await admin.goto(`${base}/admin/overview`);
  await expect(admin).toHaveURL(/\/admin\/login\?redirect=/);
  await expect(
    admin.getByRole("heading", { name: "管理后台登录" }),
  ).toBeVisible();
  const loginBounds = await admin.locator(".admin-login-page").boundingBox();
  expect(loginBounds?.x).toBe(0);
  expect(loginBounds?.width).toBe(await admin.evaluate(() => innerWidth));
  await expect(admin.getByRole("navigation", { name: "网站导航" })).toHaveCount(
    0,
  );
  await admin.getByLabel(/^(邮箱|账号)$/, { exact: true }).fill(email);
  await admin.getByLabel("密码", { exact: true }).fill(password);
  await solveCaptcha(admin);
  await admin.getByRole("button", { name: "登录", exact: true }).click();
  await expect(admin).toHaveURL(`${base}/admin/overview`);
  expect(
    (await context.cookies()).find(
      (cookie) => cookie.name === "ostool-admin-access",
    ),
  ).toMatchObject({ httpOnly: true, sameSite: "Strict" });
  expect(
    (await context.cookies()).find(
      (cookie) => cookie.name === "ostool-website-access",
    )?.value,
  ).toBe(websiteToken);
  await admin.getByRole("button", { name: "账号菜单", exact: true }).click();
  await admin.getByRole("menuitem", { name: "退出登录", exact: true }).click();
  await expect(admin).toHaveURL(`${base}/admin/login`);
  expect(
    await admin.evaluate(() =>
      localStorage.getItem("ostool.admin.access_token"),
    ),
  ).toBeNull();
  await page.reload();
  await expect(page.getByRole("heading", { name: "当前账号" })).toBeVisible();
  expect(
    (await context.cookies()).find(
      (cookie) => cookie.name === "ostool-website-access",
    )?.value,
  ).toBe(websiteToken);
  await admin.getByLabel(/^(邮箱|账号)$/, { exact: true }).fill(email);
  await admin.getByLabel("密码", { exact: true }).fill(password);
  await solveCaptcha(admin);
  await admin.getByRole("button", { name: "登录", exact: true }).click();
  await expect(admin).toHaveURL(`${base}/admin/overview`);
  await page
    .getByRole("button", { name: "用户菜单", exact: true })
    .first()
    .click();
  await page.getByRole("menuitem", { name: "退出登录", exact: true }).click();
  await expect(page).toHaveURL(`${base}/`);
  await admin.reload();
  await expect(
    admin.getByRole("heading", { name: "服务运行状态", exact: true }),
  ).toBeVisible();
  expect(
    await admin.evaluate(() => localStorage.getItem("ostool.access_token")),
  ).toBeNull();
  await admin.screenshot({
    path: "/tmp/ostool-independent-admin.png",
    animations: "disabled",
  });
  await page.screenshot({
    path: "/tmp/ostool-independent-site.png",
    animations: "disabled",
  });
  expect(errors).toEqual([]);
  await admin.close();
});

test("website content uses eighty percent width on desktops and keeps mobile gutters", async ({
  page,
}) => {
  for (const width of [1920, 2560]) {
    await page.setViewportSize({ width, height: 1000 });
    await page.goto("/");
    await expect(
      page.getByRole("navigation", { name: "网站导航" }),
    ).toBeVisible();
    for (const selector of [
      ".site-header-inner",
      ".public-hero-section",
      ".home-feature-section",
      ".site-footer-main",
    ]) {
      const element = page.locator(selector).first();
      await expect
        .poll(() =>
          element.evaluate((el) =>
            Math.round(el.getBoundingClientRect().width),
          ),
        )
        .toBe(Math.round(width * 0.8));
    }
    await page.goto("/devices");
    await expect
      .poll(() =>
        page
          .locator(".public-inner-page")
          .evaluate((el) => Math.round(el.getBoundingClientRect().width)),
      )
      .toBe(Math.round(width * 0.8));
  }
  await page.setViewportSize({ width: 390, height: 844 });
  for (const path of ["/", "/devices", "/docs"]) {
    await page.goto(path);
    await expect
      .poll(() =>
        page
          .locator(".site-header-inner")
          .evaluate((el) => Math.round(el.getBoundingClientRect().width)),
      )
      .toBe(358);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  }
  await page.screenshot({
    path: "/tmp/ostool-portal-responsive.png",
    fullPage: true,
  });
});

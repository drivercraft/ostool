import { expect, test } from "playwright/test";

test("website auth fills one viewport with an inline legal row and inset captcha", async ({
  browser,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires the disposable public fixture");
  const context = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
  });
  try {
    const page = await context.newPage();
    for (const path of ["/login", "/register"]) {
      await page.goto(base + path);
      await expect(
        page.getByLabel("图形验证码", { exact: true }),
      ).toBeEnabled();
      await expect(page.locator(".site-header")).toHaveCount(0);
      await expect(page.locator(".site-footer")).toHaveCount(0);
      const aside = await page
        .getByRole("complementary", { name: "平台介绍" })
        .boundingBox();
      const form = await page.locator(".auth-card").boundingBox();
      expect(aside!.x).toBe(0);
      expect(aside!.height).toBe(1000);
      expect(form!.width).toBeGreaterThanOrEqual(500);
      expect(form!.x).toBeGreaterThanOrEqual(aside!.width);
      const input = await page
        .getByLabel("图形验证码", { exact: true })
        .boundingBox();
      const image = await page
        .getByRole("button", { name: "刷新图形验证码" })
        .boundingBox();
      expect(image!.x).toBeGreaterThan(input!.x);
      expect(image!.x + image!.width).toBeLessThan(input!.x + input!.width);
      expect(image!.y).toBeGreaterThan(input!.y);
      expect(image!.y + image!.height).toBeLessThan(input!.y + input!.height);
      if (path === "/register") {
        const alignment = await page
          .locator(".auth-legal")
          .evaluate((element) => {
            const checkbox = element
              .querySelector('[role="checkbox"]')!
              .getBoundingClientRect();
            const label = element.querySelector("label")!;
            const range = document.createRange();
            range.setStart(label.firstChild!, 0);
            range.setEnd(label.firstChild!, 1);
            const text = range.getBoundingClientRect();
            return {
              checkboxX: checkbox.x,
              checkboxY: checkbox.y,
              textX: text.x,
              textY: text.y,
              labelHeight: label.getBoundingClientRect().height,
            };
          });
        expect(alignment.textX).toBeGreaterThan(alignment.checkboxX);
        expect(Math.abs(alignment.textY - alignment.checkboxY)).toBeLessThan(8);
        expect(alignment.labelHeight).toBeLessThan(30);
      }
      await page.screenshot({
        path: `/tmp/ostool-auth-${path.slice(1)}-desktop.png`,
      });
      await page.getByRole("button", { name: "切换网站主题" }).click();
      await page
        .getByRole("menuitemradio", { name: "黑暗", exact: true })
        .click();
      await expect(page.locator("html")).toHaveClass(/dark/);
      await page.screenshot({
        path: `/tmp/ostool-auth-${path.slice(1)}-dark.png`,
      });
      await page.getByRole("button", { name: "切换网站主题" }).click();
      await page
        .getByRole("menuitemradio", { name: "明亮", exact: true })
        .click();
    }
    for (const viewport of [
      { width: 1366, height: 768 },
      { width: 390, height: 844 },
      { width: 320, height: 568 },
    ]) {
      await page.setViewportSize(viewport);
      await page.goto(base + "/register");
      await expect(
        page.getByLabel("图形验证码", { exact: true }),
      ).toBeEnabled();
      const geometry = await page.evaluate(() => ({
        width: document.documentElement.scrollWidth,
        height: document.documentElement.scrollHeight,
        viewportWidth: innerWidth,
        viewportHeight: innerHeight,
      }));
      expect(geometry.width).toBeLessThanOrEqual(geometry.viewportWidth);
      expect(geometry.height).toBeLessThanOrEqual(geometry.viewportHeight);
      if (viewport.width >= 901) {
        const form = await page.locator(".auth-card").boundingBox();
        expect(form!.y + form!.height).toBeLessThanOrEqual(viewport.height);
      }
      await page
        .getByRole("button", { name: "创建账号", exact: true })
        .scrollIntoViewIfNeeded();
      await expect(
        page.getByRole("button", { name: "创建账号", exact: true }),
      ).toBeVisible();
      if (viewport.width < 900)
        await expect(
          page.getByRole("complementary", { name: "平台介绍" }),
        ).toBeHidden();
      await page.screenshot({
        path: `/tmp/ostool-auth-register-${viewport.width}.png`,
      });
    }
    await page.goto(base + "/admin/login");
    await expect(page.getByLabel("图形验证码", { exact: true })).toBeEnabled();
    const input = await page
      .getByLabel("图形验证码", { exact: true })
      .boundingBox();
    const image = await page
      .getByRole("button", { name: "刷新图形验证码" })
      .boundingBox();
    expect(image!.x + image!.width).toBeLessThan(input!.x + input!.width);
    expect(image!.y + image!.height).toBeLessThan(input!.y + input!.height);
  } finally {
    await context.close();
  }
});

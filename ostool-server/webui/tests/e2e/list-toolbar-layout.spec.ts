import { expect, test } from "../fixtures/local";

test("list filters hide field explanations and align all controls with the layout button", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1920, height: 1000 });
  for (const route of [
    "boards",
    "dtbs",
    "sessions",
    "session-history",
    "users",
    "users/roles",
    "rentals",
    "rental-requests",
    "audits",
    "device-documents",
  ]) {
    await page.goto(`/qcl/${route}`);
    const filters = page.locator(".listing-filters").filter({ visible: true });
    await expect(filters).toHaveCount(1);
    const layout = filters.getByRole("button", {
      name: "切换列表布局",
      exact: true,
    });
    await expect(layout).toBeVisible();
    await expect(filters.locator(".field-help-trigger:visible")).toHaveCount(0);
    await page.evaluate(() => document.fonts.ready);
    const controls = await filters.evaluate((element) =>
      Array.from(
        element.querySelectorAll(
          "input, select, button:not(.field-help-trigger)",
        ),
      )
        .filter((control) => control.getBoundingClientRect().height > 0)
        .map((control) => {
          const box = control.getBoundingClientRect();
          return { top: box.y, height: box.height };
        }),
    );
    expect(controls.length).toBeGreaterThan(1);
    for (const control of controls) {
      expect(Math.abs(control.top - controls[0].top), route).toBeLessThan(1);
      expect(control.height, route).toBe(controls[0].height);
    }
    if (route === "boards") {
      await expect(
        filters.getByLabel("搜索开发板", { exact: true }),
      ).toBeVisible();
      await expect(filters.getByLabel("板型", { exact: true })).toBeVisible();
      await page.screenshot({
        path: "/tmp/ostool-list-toolbar-1920.png",
        fullPage: true,
      });
    }
  }
  await page.goto("/qcl/boards/new");
  await expect(
    page.getByRole("button", { name: "板型说明", exact: true }),
  ).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/qcl/boards");
  await expect(
    page.locator(".listing-filters .field-help-trigger:visible"),
  ).toHaveCount(0);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

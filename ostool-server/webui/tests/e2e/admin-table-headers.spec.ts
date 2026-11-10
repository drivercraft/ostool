import { test, expect } from "../fixtures/local";

test("administrator tables share restrained headers in light and dark themes", async ({
  page,
  request,
}) => {
  const id = `table-style-${Date.now()}`;
  const me = await (
    await request.get("http://127.0.0.1:4175/api/v1/auth/me")
  ).json();
  expect(
    (
      await request.post("http://127.0.0.1:4175/api/v1/admin/boards", {
        data: {
          id,
          board_type: "table-style",
          boot: { kind: "pxe" },
          power_management: {
            kind: "custom",
            power_on_cmd: "true",
            power_off_cmd: "true",
          },
        },
      })
    ).status(),
  ).toBe(201);
  expect(
    (
      await request.post("http://127.0.0.1:4175/api/v1/admin/rentals", {
        data: {
          board_id: id,
          user_id: me.id,
          starts_at: new Date(Date.now() - 60000).toISOString(),
          ends_at: new Date(Date.now() + 3600000).toISOString(),
          notes: "table style fixture",
        },
      })
    ).status(),
  ).toBe(201);
  try {
    let reference: Record<string, string> | undefined;
    for (const theme of ["light", "dark"]) {
      for (const route of [
        "/admin/boards",
        "/admin/users",
        "/admin/rentals",
        "/admin/device-documents",
      ]) {
        await page.goto(route);
        await expect(page.getByRole("columnheader").first()).toBeVisible();
        await page
          .getByRole("button", { name: "切换主题", exact: true })
          .click();
        await page
          .getByRole("menuitemradio", {
            name: theme === "dark" ? "黑暗" : "明亮",
            exact: true,
          })
          .click();
        await expect(page.locator("html")).toHaveClass(
          theme === "dark" ? /dark/ : /light/,
        );
        await expect(
          page.getByRole("menuitemradio", {
            name: theme === "dark" ? "黑暗" : "明亮",
            exact: true,
          }),
        ).toBeHidden();
        const styles = await page
          .getByRole("columnheader")
          .first()
          .evaluate((el) => {
            const css = getComputedStyle(el);
            return {
              size: css.fontSize,
              weight: css.fontWeight,
              paddingLeft: css.paddingLeft,
              paddingRight: css.paddingRight,
              height: css.height,
            };
          });
        reference ??= styles;
        expect(styles).toEqual(reference);
        expect(Number.parseFloat(styles.height)).toBeGreaterThanOrEqual(48);
        if (route === "/admin/boards")
          await page.screenshot({
            path: `/tmp/ostool-admin-table-header-${theme}.png`,
            fullPage: true,
          });
      }
    }
  } finally {
    const rentals = await (
      await request.get(
        `http://127.0.0.1:4175/api/v1/admin/rentals?board=${encodeURIComponent(id)}`,
      )
    ).json();
    for (const rental of rentals.items ?? [])
      if (rental.board_id === id)
        await request.delete(
          `http://127.0.0.1:4175/api/v1/admin/rentals/${rental.id}`,
        );
    await request.delete(
      `http://127.0.0.1:4175/api/v1/admin/boards/${encodeURIComponent(id)}`,
    );
  }
});

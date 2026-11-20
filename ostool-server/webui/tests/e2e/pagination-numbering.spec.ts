import { expect, test } from "../fixtures/local";

const base = "http://127.0.0.1:4175";
test("all list pages share pagination alignment and continuous row numbers", async ({
  page,
  request,
}) => {
  test.setTimeout(120000);
  const ids = Array.from(
    { length: 25 },
    (_, index) => `pagination-board-${String(index + 1).padStart(2, "0")}`,
  );
  let sessionId = "";
  try {
    for (const id of ids) {
      const result = await request.post(base + "/api/v1/admin/boards", {
        data: {
          id,
          board_type: "pagination-fixture",
          disabled: false,
          boot: { kind: "pxe" },
          power_management: {
            kind: "custom",
            power_on_cmd: "true",
            power_off_cmd: "true",
          },
        },
      });
      expect(result.status()).toBe(201);
    }
    // Real session lifecycle records, using only fixture-owned boards and harmless power commands.
    for (let index = 0; index < 25; index++) {
      const result = await request.post(base + "/api/v1/sessions", {
        data: {
          board_type: "pagination-fixture",
          board_id: ids[0],
          client_name: "pagination-client",
        },
      });
      expect(result.status()).toBe(201);
      sessionId = (await result.json()).session_id;
      expect(
        (
          await request.delete(base + "/api/v1/admin/sessions/" + sessionId)
        ).status(),
      ).toBe(202);
      await expect
        .poll(async () =>
          (
            await (await request.get(base + "/api/v1/admin/sessions")).json()
          ).sessions.some((entry: { id: string }) => entry.id === sessionId),
        )
        .toBe(false);
      sessionId = "";
    }
    for (const route of [
      "boards",
      "device-documents",
      "dtbs",
      "users",
      "users/roles",
      "rentals",
      "rental-requests",
      "sessions",
      "session-history",
      "audits",
    ]) {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(`/qcl/${route}`);
      await expect(page.getByRole("columnheader").first()).toHaveText("编号");
      const pagination = page.getByRole("navigation", {
        name: "分页",
        exact: true,
      });
      await expect(pagination).toHaveCount(1);
      await expect(pagination.locator(".pagination-summary")).not.toHaveText(
        "正在同步…",
      );
      await page.evaluate(() => document.fonts.ready);
      const controls = pagination.locator(
        ".pagination-actions select, .pagination-actions button",
      );
      const boxes = await controls.evaluateAll((elements) =>
        elements.map((element) => {
          const box = element.getBoundingClientRect();
          return { y: box.y, height: box.height };
        }),
      );
      expect(boxes).toHaveLength(3);
      for (const box of boxes) {
        expect(Math.abs(box.y - boxes[0].y), route).toBeLessThan(1);
        expect(box.height, route).toBe(boxes[0].height);
      }
      await expect(pagination.locator(".field-title-row")).toHaveCount(0);
      const rows = page.locator("tbody tr");
      if (
        (await rows.count()) &&
        !(await page.locator('[aria-label="正在加载数据"]').count())
      ) {
        const values = await rows.locator("td:first-child").allTextContents();
        expect(
          values.every((value, index) => value === String(index + 1)),
          route,
        ).toBe(true);
      }
      if (
        ["boards", "device-documents", "session-history", "audits"].includes(
          route,
        )
      ) {
        await pagination
          .getByRole("button", { name: "下一页", exact: true })
          .click();
        await expect(pagination.locator(".pagination-position")).toContainText(
          "第 2",
        );
        await expect(pagination.locator(".pagination-summary")).not.toHaveText(
          "正在同步…",
        );
        await expect(rows.first().locator("td").first()).toHaveText("21");
        await pagination
          .getByRole("combobox", { name: "每页条数" })
          .selectOption("10");
        await expect(pagination.locator(".pagination-position")).toContainText(
          "第 1",
        );
        await expect(pagination.locator(".pagination-summary")).not.toHaveText(
          "正在同步…",
        );
        await expect(rows.first().locator("td").first()).toHaveText("1");
        await pagination
          .getByRole("button", { name: "下一页", exact: true })
          .click();
        await expect(pagination.locator(".pagination-summary")).not.toHaveText(
          "正在同步…",
        );
        await expect(rows.first().locator("td").first()).toHaveText("11");
        if (route === "session-history")
          await page.screenshot({
            path: "/tmp/ostool-history-pagination-1440.png",
            fullPage: true,
          });
        await page.getByRole("button", { name: "切换列表布局" }).click();
        await page.getByRole("menuitemradio", { name: /卡片视图/ }).click();
        await expect(
          page.getByRole("listitem").first().locator(".data-card-number"),
        ).toHaveText("编号 11");
      }
      await page.setViewportSize({ width: 390, height: 844 });
      await expect(
        pagination.getByRole("combobox", { name: "每页条数" }),
      ).toBeVisible();
      expect(
        await pagination.evaluate(
          (element) => element.scrollWidth <= element.clientWidth,
        ),
        route,
      ).toBe(true);
      expect(
        await page
          .locator(".panel-footer")
          .evaluate(
            (element) =>
              element.getBoundingClientRect().bottom <= innerHeight + 1,
          ),
        route,
      ).toBe(true);
    }
  } finally {
    if (sessionId)
      await request.delete(base + "/api/v1/admin/sessions/" + sessionId);
    for (const id of ids)
      await request.delete(base + "/api/v1/admin/boards/" + id);
  }
});

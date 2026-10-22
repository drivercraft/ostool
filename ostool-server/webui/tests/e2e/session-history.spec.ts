import { expect, test } from "playwright/test";

test("lease history follows real allocation and release through the paginated event stream", async ({
  page,
  request,
}) => {
  const base = "http://127.0.0.1:4175";
  const board = "history-browser-board";
  expect(
    (
      await request.post(base + "/api/v1/admin/boards", {
        data: {
          id: board,
          board_type: "history-browser",
          tags: [],
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
  ).toBe(true);
  const created = await request.post(base + "/api/v1/sessions", {
    data: {
      board_type: "history-browser",
      client_name: "history-browser-client",
    },
  });
  expect(created.status()).toBe(201);
  const session = (await created.json()).session_id;
  const privateReads: string[] = [];
  page.on("request", (r) => {
    if (
      r.method() === "GET" &&
      r.url().includes("/api/v1/admin/session-history")
    )
      privateReads.push(r.url());
  });
  await page.goto("/admin/sessions");
  await page.getByRole("link", { name: "租约历史", exact: true }).click();
  await page.getByLabel("设备标识", { exact: true }).fill(board);
  const row = page.getByRole("row").filter({ hasText: session });
  await expect(row).toBeVisible();
  await expect(row.getByText("使用中", { exact: true })).toBeVisible();
  expect(
    (await request.delete(base + "/api/v1/admin/sessions/" + session)).status(),
  ).toBe(202);
  await expect(row.getByText("管理员释放", { exact: true })).toBeVisible();
  await expect(row.getByText("已结束", { exact: true })).toBeVisible();
  await row.getByRole("button", { name: "租约详情", exact: true }).click();
  await expect(page.getByRole("dialog")).toContainText(session);
  await expect(page.getByRole("dialog")).toContainText("最后心跳");
  await page.keyboard.press("Escape");
  await page
    .getByLabel("结束原因", { exact: true })
    .selectOption("client_released");
  await expect(page.getByRole("status", { name: "暂无数据" })).toBeVisible();
  await page
    .getByLabel("结束原因", { exact: true })
    .selectOption("admin_released");
  await expect(row).toBeVisible();
  await page.getByRole("button", { name: "切换列表布局" }).click();
  await page.getByRole("menuitemradio", { name: /卡片视图/ }).click();
  const card = page.getByRole("listitem").filter({ hasText: session });
  await expect(card).toContainText("管理员释放");
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  expect(privateReads).toEqual([]);
  await request.delete(base + "/api/v1/admin/boards/" + board);
  // Deleting the inventory entry leaves the immutable device identifier in history.
  await page.reload();
  await expect(
    page.getByRole("listitem").filter({ hasText: session }),
  ).toBeVisible();
});

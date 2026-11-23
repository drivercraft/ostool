import { expect, test } from "../fixtures/local";
const base = "http://127.0.0.1:4175";

test("administrators can remove failed sessions without an offline device recovering", async ({
  page,
  request,
}) => {
  test.setTimeout(60000);
  const id = "force-release-offline-board";
  let session = "";
  const create = await request.post(base + "/api/v1/admin/boards", {
    data: {
      id,
      board_type: id,
      disabled: false,
      boot: { kind: "pxe" },
      power_management: {
        kind: "custom",
        power_on_cmd: "true",
        power_off_cmd: "false",
      },
    },
  });
  expect(create.status()).toBe(201);
  const allocate = async () => {
    const response = await request.post(base + "/api/v1/sessions", {
      data: { board_type: id, client_name: "offline-fixture" },
    });
    expect(response.status()).toBe(201);
    session = (await response.json()).session_id;
    expect(
      (
        await request.delete(base + "/api/v1/admin/sessions/" + session)
      ).status(),
    ).toBe(202);
  };
  try {
    await allocate();
    await page.goto("/qcl/sessions");
    const row = page.getByRole("row").filter({ hasText: session });
    await expect(row.getByText("释放失败", { exact: true })).toBeVisible();
    await expect(
      row.getByRole("button", { name: "释放会话", exact: true }),
    ).toBeDisabled();
    await expect(
      row.getByRole("button", { name: "强制释放", exact: true }),
    ).toBeEnabled();
    await row.getByRole("button", { name: "强制释放", exact: true }).click();
    await expect(page.getByRole("dialog")).toContainText("不等待设备恢复");
    await page.getByRole("button", { name: "确认", exact: true }).click();
    await expect(row).toHaveCount(0);
    session = "";
    await page.goto("/qcl/boards");
    const board = page.getByRole("row").filter({ hasText: id });
    await expect(board.getByText("已禁用", { exact: true })).toBeVisible();
    await page.goto("/qcl/session-history");
    await page.getByLabel("设备标识", { exact: true }).fill(id);
    await expect(
      page.locator("tbody").getByText("管理员强制结束", { exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: "/tmp/ostool-force-release-history.png",
      fullPage: true,
    });
    // Explicitly re-enable the fixture, then verify the equivalent action in cards.
    expect(
      (
        await request.patch(base + "/api/v1/admin/boards/" + id, {
          data: { disabled: false },
        })
      ).status(),
    ).toBe(200);
    await allocate();
    await page.goto("/qcl/boards");
    await board.getByRole("button", { name: "更多", exact: true }).click();
    await expect(
      page.getByRole("menuitem", { name: "强制释放", exact: true }),
    ).toBeEnabled();
    await page.keyboard.press("Escape");
    await page.getByRole("button", { name: "切换列表布局" }).click();
    await page.getByRole("menuitemradio", { name: /卡片视图/ }).click();
    const card = page.getByRole("listitem").filter({ hasText: id });
    await card.getByRole("button", { name: "更多", exact: true }).click();
    await expect(
      page.getByRole("menuitem", { name: "删除", exact: true }),
    ).toBeEnabled();
    await page.getByRole("menuitem", { name: "删除", exact: true }).click();
    await expect(page.getByRole("dialog")).toContainText("强制结束其会话");
    const deleted = page.waitForResponse(
      (response) =>
        response.request().method() === "DELETE" &&
        response.url().includes(`/api/v1/admin/boards/${id}?force=true`),
    );
    await page.getByRole("button", { name: "确认", exact: true }).click();
    expect((await deleted).status()).toBe(204);
    await expect(card).toHaveCount(0);
    expect(
      (await request.get(base + "/api/v1/admin/boards/" + id)).status(),
    ).toBe(404);
    await expect
      .poll(
        async () =>
          (await (await request.get(base + "/api/v1/admin/sessions")).json())
            .sessions.length,
      )
      .toBe(0);
    session = "";
  } finally {
    if (session)
      await request.delete(
        base + "/api/v1/admin/sessions/" + session + "?force=true",
      );
    await request.delete(base + "/api/v1/admin/boards/" + id + "?force=true");
  }
});

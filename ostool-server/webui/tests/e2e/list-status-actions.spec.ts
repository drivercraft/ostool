import { expect, test } from "../fixtures/local";

const base = "http://127.0.0.1:4175";
const power = { kind: "custom", power_on_cmd: "true", power_off_cmd: "true" };

test("board, user and role lists toggle status through SSE without replacing other fields", async ({
  page,
  request,
}) => {
  const board = "quick-status-board";
  let userId = "";
  let roleId = "";
  const created = await request.post(base + "/api/v1/admin/boards", {
    data: {
      id: board,
      board_type: "quick-status",
      disabled: true,
      tags: ["preserve"],
      notes: "保留备注",
      boot: { kind: "pxe" },
      power_management: power,
    },
  });
  expect(created.status()).toBe(201);
  const role = await request.post(base + "/api/v1/admin/roles", {
    data: {
      code: "quick_status",
      name: "快捷状态角色",
      description: "保留角色说明",
      permissions: ["devices.read"],
      enabled: true,
    },
  });
  expect(role.status()).toBe(201);
  roleId = (
    await (await request.get(base + "/api/v1/admin/roles")).json()
  ).find((entry: { code: string }) => entry.code === "quick_status").id;
  try {
    await page.goto("/qcl/boards");
    let row = page.getByRole("row").filter({ hasText: board });
    await row.getByRole("button", { name: "启用", exact: true }).click();
    await expect(row.getByText("空闲", { exact: true })).toBeVisible();
    await row.getByRole("button", { name: "禁用", exact: true }).click();
    await expect(row.getByText("已禁用", { exact: true })).toBeVisible();
    const saved = await (
      await request.get(base + "/api/v1/admin/boards/" + board)
    ).json();
    expect(saved.notes).toBe("保留备注");
    expect(saved.tags).toEqual(["preserve"]);
    expect(saved.power_management).toEqual(power);
    await page.getByRole("button", { name: "切换列表布局" }).click();
    await page.getByRole("menuitemradio", { name: /卡片视图/ }).click();
    const card = page.getByRole("listitem").filter({ hasText: board });
    await card.getByRole("button", { name: "启用", exact: true }).click();
    await expect(card.getByText("空闲", { exact: true })).toBeVisible();

    await page.goto("/qcl/users/roles");
    row = page.getByRole("row").filter({ hasText: "快捷状态角色" });
    await row.getByRole("button", { name: "禁用", exact: true }).click();
    await expect(row.getByText("已停用", { exact: true })).toBeVisible();
    await row.getByRole("button", { name: "启用", exact: true }).click();
    await expect(row.getByText("已启用", { exact: true })).toBeVisible();
    const roles = await (
      await request.get(base + "/api/v1/admin/roles")
    ).json();
    const changed = roles.find((entry: { id: string }) => entry.id === roleId);
    expect(changed.description).toBe("保留角色说明");
    expect(changed.permissions).toContain("devices.read");
    await expect(
      page
        .getByRole("row")
        .filter({ has: page.getByRole("cell", { name: "admin", exact: true }) })
        .getByRole("button", { name: "禁用", exact: true }),
    ).toBeDisabled();

    const user = await request.post(base + "/api/v1/admin/users", {
      data: {
        username: "quick_status_user",
        email: "quick-status@test.local",
        display_name: "快捷用户",
        role_id: roleId,
        disabled: false,
        password: "quick-status-password-123",
      },
    });
    expect(user.status()).toBe(201);
    userId = (
      await (
        await request.get(base + "/api/v1/admin/users?q=quick_status_user")
      ).json()
    ).items.find(
      (entry: { username: string }) => entry.username === "quick_status_user",
    ).id;
    await page.goto("/qcl/users");
    row = page.getByRole("row").filter({ hasText: "quick_status_user" });
    await row.getByRole("button", { name: "禁用", exact: true }).click();
    await expect(row.getByText("已禁用", { exact: true })).toBeVisible();
    await row.getByRole("button", { name: "启用", exact: true }).click();
    await expect(row.getByText("正常", { exact: true })).toBeVisible();
    await expect(row).toContainText("快捷用户");
    await expect(row).toContainText("快捷状态角色");
    await expect(
      page
        .getByRole("row")
        .filter({ hasText: "local-browser-admin@test.local" })
        .getByRole("button", { name: "禁用", exact: true }),
    ).toBeDisabled();
  } finally {
    if (userId) await request.delete(base + "/api/v1/admin/users/" + userId);
    if (roleId) await request.delete(base + "/api/v1/admin/roles/" + roleId);
    await request.delete(base + "/api/v1/admin/boards/" + board);
  }
});

test("current sessions show one meaningful state without the default device-wait badge", async ({
  page,
  request,
}) => {
  await page.setViewportSize({ width: 1680, height: 900 });
  const board = "session-status-board";
  const created = await request.post(base + "/api/v1/admin/boards", {
    data: {
      id: board,
      board_type: "session-status",
      disabled: false,
      boot: { kind: "pxe" },
      power_management: power,
    },
  });
  expect(created.status()).toBe(201);
  let session = "";
  try {
    const allocated = await request.post(base + "/api/v1/sessions", {
      data: { board_type: "session-status", client_name: "status-client" },
    });
    expect(allocated.status()).toBe(201);
    session = (await allocated.json()).session_id;
    await page.goto("/qcl/boards");
    await expect(
      page
        .getByRole("row")
        .filter({ hasText: board })
        .getByRole("button", { name: "禁用", exact: true }),
    ).toBeDisabled();
    await page.goto("/qcl/sessions");
    const row = page.getByRole("row").filter({ hasText: session });
    await expect(row.getByText("占用中", { exact: true })).toBeVisible();
    await expect(row.getByText("终端未连接", { exact: true })).toBeVisible();
    await expect(row.locator('[data-slot="badge"]')).toHaveCount(1);
    await expect(row.getByText("等待设备", { exact: true })).toHaveCount(0);
    await row.getByRole("button", { name: "会话状态详情说明" }).click();
    await expect(page.getByRole("note")).toContainText("终端通道未连接");
    await page.keyboard.press("Escape");
    await expect(page.getByRole("note")).toBeHidden();
    await page.screenshot({
      path: "/tmp/ostool-session-status.png",
      fullPage: true,
    });
  } finally {
    if (session) {
      await request.delete(base + "/api/v1/admin/sessions/" + session);
      await expect
        .poll(async () =>
          (
            await (await request.get(base + "/api/v1/admin/sessions")).json()
          ).sessions.some((entry: { id: string }) => entry.id === session),
        )
        .toBe(false);
    }
    await request.delete(base + "/api/v1/admin/boards/" + board);
  }
});

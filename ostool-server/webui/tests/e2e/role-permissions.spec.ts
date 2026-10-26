import { expect, test } from "playwright/test";
import { apiLogin } from "../fixtures/security";

test("role matrix grants instant device access without administrator rights and enforces a separate read-only console role", async ({
  page,
  context,
  browser,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires disposable public fixtures");
  await apiLogin(
    context.request,
    base!,
    process.env.OSTOOL_E2E_ADMIN_EMAIL!,
    process.env.OSTOOL_E2E_ADMIN_PASSWORD!,
  );
  const headers = { origin: base!, "x-ostool-surface": "admin" };
  const stamp = Date.now();
  const name = `即时研发角色 ${stamp}`;
  await page.goto(base + "/admin/users/roles");
  await page.getByRole("button", { name: "创建角色", exact: true }).click();
  await expect(page).toHaveURL(base + "/admin/users/roles/new");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByLabel("角色编码", { exact: true }).fill(`engineer_${stamp}`);
  await expect(
    page.getByRole("checkbox", { name: "启用角色", exact: true }),
  ).toBeChecked();
  await page.getByLabel("角色名称", { exact: true }).fill(name);
  await page
    .getByRole("checkbox", { name: "即时分配设备", exact: true })
    .check();
  await expect(
    page.getByRole("checkbox", { name: "使用已分配设备", exact: true }),
  ).toBeChecked();
  await expect(
    page.getByRole("checkbox", { name: "进入管理后台", exact: true }),
  ).not.toBeChecked();
  await page.getByLabel("单次最长使用（分钟）", { exact: true }).fill("60");
  await page.getByLabel("每用户并发设备数", { exact: true }).fill("2");
  await page.getByLabel("最短可用时长（分钟）", { exact: true }).fill("5");
  await page.getByLabel("可用板型范围", { exact: true }).fill("permission-arm");
  await page.getByRole("button", { name: "保存角色", exact: true }).click();
  await expect(page).toHaveURL(base + "/admin/users/roles");
  await expect(page.getByText(name, { exact: true })).toBeVisible();
  const roles = await (
    await context.request.get(base + "/api/v1/admin/roles", { headers })
  ).json();
  const role = roles.find((r: { name: string }) => r.name === name);
  expect(role.permissions).toEqual(
    expect.arrayContaining(["board.use", "board.instant"]),
  );
  expect(role.administrator).toBe(false);
  expect(role.code).toBe(`engineer_${stamp}`);
  expect(role.enabled).toBe(true);
  await page
    .getByText(name, { exact: true })
    .locator("xpath=ancestor::tr")
    .getByRole("link", { name: "编辑", exact: true })
    .click();
  await expect(page.getByLabel("角色编码", { exact: true })).toBeDisabled();
  await page.getByRole("link", { name: "取消", exact: true }).click();
  const username = `engineer_${stamp}`,
    email = username + "@test.local",
    password = "permission-browser-password-123";
  const user = await context.request.post(base + "/api/v1/admin/users", {
    headers,
    data: {
      username,
      email,
      password,
      display_name: "研发工程师",
      role_id: role.id,
      disabled: false,
    },
  });
  expect(user.ok()).toBe(true);
  const id = `permission-board-${stamp}`;
  expect(
    (
      await context.request.post(base + "/api/v1/admin/boards", {
        headers,
        data: {
          id,
          board_type: "permission-arm",
          allow_instant_use: true,
          disabled: false,
          tags: [],
          serial: null,
          notes: null,
          network_identity: { mac_address: "02:19:28:37:46:55" },
          boot: { kind: "httpboot", boot_arch: "aarch64" },
          power_management: {
            kind: "custom",
            power_on_cmd: "true",
            power_off_cmd: "true",
          },
        },
      })
    ).ok(),
  ).toBe(true);
  const member = await browser.newContext();
  try {
    await apiLogin(member.request, base!, email, password, false);
    const me = await (
      await member.request.get(base + "/api/v1/auth/me")
    ).json();
    expect(me.can_access_admin).toBe(false);
    expect(me.permissions).toContain("board.instant");
    expect([401, 403]).toContain(
      (await member.request.get(base + "/api/v1/admin/boards")).status(),
    );
    const session = await member.request.post(base + "/api/v1/sessions", {
      headers: { origin: base! },
      data: { board_type: "permission-arm" },
    });
    expect(session.status(), await session.text()).toBe(201);
    const allocated = await session.json();
    expect(allocated.board_id).toBe(id);
    expect(
      (
        await member.request.put(
          base + `/api/v1/sessions/${allocated.session_id}/files`,
          {
            headers: {
              origin: base!,
              "X-File-Path": "kernel.txt",
              "content-type": "application/octet-stream",
            },
            data: Buffer.from("authenticated instant session"),
          },
        )
      ).status(),
    ).toBe(201);
    const download = await member.request.get(
      base + `/share/sessions/${allocated.session_id}/kernel.txt`,
    );
    expect(await download.text()).toBe("authenticated instant session");
    const memberPage = await member.newPage();
    await memberPage.goto(base + "/account");
    await expect(
      memberPage.getByText("即时使用设备", { exact: true }),
    ).toBeVisible();
    const socket = await memberPage.evaluate(
      async ({ base, sid }) =>
        new Promise<string>((resolve, reject) => {
          const ws = new WebSocket(
            base.replace(/^http/, "ws") + `/api/v1/sessions/${sid}/serial/ws`,
          );
          const timer = setTimeout(() => {
            ws.close();
            reject(new Error("websocket timeout"));
          }, 5000);
          ws.onopen = () => {
            clearTimeout(timer);
            ws.send(JSON.stringify({ type: "close" }));
            ws.close();
            resolve("connected");
          };
          ws.onerror = () => {
            clearTimeout(timer);
            reject(new Error("instant websocket denied"));
          };
        }),
      { base: base!, sid: allocated.session_id },
    );
    expect(socket).toBe("connected");
    await expect
      .poll(async () => {
        const data = await (
          await member.request.get(base + "/api/v1/account/rentals")
        ).json();
        return data.items.find((r: { board_id: string }) => r.board_id === id)
          ?.released_at;
      })
      .toBeTruthy();
  } finally {
    await member.close();
  }
  const viewerName = `设备观察角色 ${stamp}`;
  expect(
    (
      await context.request.post(base + "/api/v1/admin/roles", {
        headers,
        data: {
          code: `viewer_${stamp}`,
          enabled: true,
          name: viewerName,
          description: "只读设备",
          permissions: ["devices.read"],
        },
      })
    ).ok(),
  ).toBe(true);
  const latest = await (
    await context.request.get(base + "/api/v1/admin/roles", { headers })
  ).json();
  const viewerRole = latest.find(
    (r: { name: string }) => r.name === viewerName,
  );
  const viewerEmail = `viewer_${stamp}@test.local`;
  expect(
    (
      await context.request.post(base + "/api/v1/admin/users", {
        headers,
        data: {
          username: `viewer_${stamp}`,
          email: viewerEmail,
          password,
          display_name: "设备观察员",
          role_id: viewerRole.id,
          disabled: false,
        },
      })
    ).ok(),
  ).toBe(true);
  const viewer = await browser.newContext();
  try {
    await apiLogin(viewer.request, base!, viewerEmail, password, true);
    const view = await viewer.newPage();
    await view.goto(base + "/admin");
    await expect(view).toHaveURL(base + "/admin/boards");
    await expect(view.getByText(id, { exact: true })).toBeVisible();
    await expect(
      view.getByRole("link", { name: "用户列表", exact: true }),
    ).toHaveCount(0);
    await expect(
      view.getByRole("link", { name: "系统设置", exact: true }),
    ).toHaveCount(0);
    await expect(
      view.getByRole("link", { name: "新增开发板", exact: true }),
    ).toHaveCount(0);
    await expect(
      view.getByRole("button", { name: "删除", exact: true }),
    ).toHaveCount(0);
    expect(
      (await viewer.request.get(base + "/api/v1/admin/users")).status(),
    ).toBe(403);
    await view.goto(base + "/admin/users");
    await expect(
      view.getByText("当前角色没有此页面的访问权限。", { exact: true }),
    ).toBeVisible();
  } finally {
    await viewer.close();
  }
});

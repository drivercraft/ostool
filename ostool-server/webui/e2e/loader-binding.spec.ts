import { expect, test } from "playwright/test";
import dgram from "node:dgram";
import { mkdir } from "node:fs/promises";
const mac = "02:00:00:00:00:42";
async function discover() {
  const socket = dgram.createSocket("udp4");
  try {
    return await new Promise<{ registration_id: string }>((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error("discovery timed out")),
        3000,
      );
      socket.once("message", (message) => {
        clearTimeout(timer);
        resolve(JSON.parse(message.toString()));
      });
      socket.send(
        Buffer.from(
          JSON.stringify({
            protocol_version: 2,
            mac_address: mac,
            current_mac_address: mac,
            arch: "x86_64",
            loader_version: "e2e",
          }),
        ),
        2998,
        "127.0.0.1",
        (e) => {
          if (e) {
            clearTimeout(timer);
            reject(e);
          }
        },
      );
    });
  } finally {
    socket.close();
  }
}
test("unbound power -> real discovery -> manual MAC binding; push preserves focus and no GET polling", async ({
  page,
  context,
  request,
}) => {
  test.skip(
    process.env.OSTOOL_TEST_DISABLE_DISCOVERY === "true",
    "UDP discovery is disabled for this isolated layout run",
  );
  const consoleErrors: string[] = [];
  page.on("pageerror", (e) => consoleErrors.push(e.message));
  const reads: string[] = [];
  page.on("request", (r) => {
    if (
      r.method() === "GET" &&
      new URL(r.url()).pathname.startsWith("/api/v1/")
    )
      reads.push(r.url());
  });
  await page.goto("/admin/boards/new");
  await expect(
    page.getByRole("status").filter({ hasText: "实时已连接" }),
  ).toBeVisible();
  await page.getByLabel("上电命令", { exact: true }).fill("true");
  await page.getByLabel("下电命令", { exact: true }).fill("true");
  await page.getByRole("button", { name: "上电", exact: true }).click();
  await expect(page.getByText("上电命令已完成")).toBeVisible();
  expect(
    await (
      await request.get("http://127.0.0.1:4175/api/v1/admin/boards")
    ).json(),
  ).toEqual([]);
  await page.getByLabel("板型", { exact: true }).fill("qemu-x86_64");
  await page.getByLabel("ID", { exact: true }).fill("e2e-board");
  await page.getByLabel("启动方式").selectOption("httpboot");
  await expect(page.getByLabel("波特率", { exact: true })).toHaveCount(0);
  await expect(page.getByLabel("启用串口")).toHaveCount(0);
  await expect(page.getByText("自动串口", { exact: true })).toBeVisible();
  const macInput = page.getByLabel("MAC 地址", { exact: true });
  await macInput.fill("02:00:00:00:00:");
  await macInput.focus();
  await page.evaluate(async () => {
    await document.fonts.ready;
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
  });
  const before = await macInput.evaluate((e) => ({
    element: e.tagName,
    scroll: window.scrollY,
  }));
  const offer = await discover();
  const poll = await request.post("http://127.0.0.1:4175/api/v1/loaders/poll", {
    data: {
      protocol_version: 2,
      registration_id: offer.registration_id,
      mac_address: mac,
      current_mac_address: mac,
      ip_address: "10.77.0.142",
      arch: "x86_64",
      loader_version: "e2e",
      hardware: {
        manufacturer: "QEMU",
        product: "Standard PC (Q35)",
        version: "q35",
        serial: "virtual-42",
      },
    },
  });
  expect(poll.ok()).toBeTruthy();
  await expect(
    page.getByRole("button", { name: "发现 1 台设备" }),
  ).toBeVisible();
  await expect(macInput).toHaveValue("02:00:00:00:00:");
  await expect(macInput).toBeFocused();
  expect(await page.evaluate(() => window.scrollY)).toBe(before.scroll);
  await macInput.fill(mac);
  await page.getByRole("button", { name: "保存开发板" }).click();
  await expect(page).toHaveURL(/boards\/e2e-board$/);
  const saved = await (
    await request.get("http://127.0.0.1:4175/api/v1/admin/boards/e2e-board")
  ).json();
  expect(saved.serial).toBeNull();
  const peer = await context.newPage();
  await peer.goto("/admin/boards");
  await expect(peer.getByText(/1 份不兼容配置已备份隔离/)).toBeVisible();
  await expect(
    peer.getByRole("link", { name: "e2e-board", exact: true }),
  ).toBeVisible();
  await page.getByLabel("备注", { exact: true }).fill("local draft");
  const current = await (
    await request.get("http://127.0.0.1:4175/api/v1/admin/boards/e2e-board")
  ).json();
  await request.put("http://127.0.0.1:4175/api/v1/admin/boards/e2e-board", {
    data: { ...current, notes: "changed by peer" },
  });
  await expect(page.getByText(/你的草稿已保留/)).toBeVisible();
  await expect(page.getByLabel("备注", { exact: true })).toHaveValue(
    "local draft",
  );
  await expect(page.getByRole("button", { name: "保存开发板" })).toBeDisabled();
  await page.getByRole("button", { name: "载入服务器版本" }).click();
  await expect(page.getByLabel("备注", { exact: true })).toHaveValue(
    "changed by peer",
  );
  await peer.getByRole("tab", { name: /发现设备/ }).click();
  // No new loader requests: the server deadline itself must publish offline.
  await expect(
    peer.getByRole("table").getByText("离线", { exact: true }),
  ).toBeVisible({
    timeout: 15_000,
  });
  expect(
    reads.every(
      (url) =>
        url.endsWith("/api/v1/admin/events") ||
        url.endsWith("/api/v1/auth/config"),
    ),
  ).toBeTruthy();
  expect(
    reads.filter((url) => url.endsWith("/api/v1/auth/config")),
  ).toHaveLength(1);
  expect(consoleErrors).toEqual([]);
  await mkdir("/tmp/ostool-admin-screenshots", { recursive: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.screenshot({
    path: "/tmp/ostool-admin-screenshots/editor.png",
    fullPage: true,
    animations: "disabled",
  });
  await peer.setViewportSize({ width: 1440, height: 900 });
  await peer.getByRole("tab", { name: /已配置/ }).click();
  await peer.screenshot({
    path: "/tmp/ostool-admin-screenshots/boards.png",
    fullPage: true,
    animations: "disabled",
  });
  await peer.close();
});
test("all management modules, DTB CRUD, session release and narrow viewport", async ({
  page,
  request,
}) => {
  const consoleErrors: string[] = [];
  page.on("pageerror", (e) => consoleErrors.push(e.message));
  await page.goto("/admin/overview");
  await expect(
    page.getByRole("link", { name: "可用开发板", exact: false }),
  ).toBeVisible();
  await page.getByRole("link", { name: "DTB 管理", exact: true }).click();
  await page.getByRole("button", { name: "上传 DTB", exact: true }).click();
  await page.getByLabel("文件", { exact: true }).setInputFiles({
    name: "e2e.dtb",
    mimeType: "application/octet-stream",
    buffer: Buffer.from("test-dtb"),
  });
  await page.getByRole("button", { name: "上传", exact: true }).click();
  await expect(
    page.getByRole("cell", { name: "e2e.dtb", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "修改", exact: true }).click();
  await page.getByLabel("文件名", { exact: true }).fill("renamed.dtb");
  await page.getByRole("button", { name: "保存修改" }).click();
  await expect(
    page.getByRole("cell", { name: "renamed.dtb", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "删除", exact: true }).click();
  await page.getByRole("button", { name: "确认", exact: true }).click();
  await expect(
    page.getByRole("cell", { name: "renamed.dtb", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("link", { name: "会话租约", exact: true }).click();
  await request.post("http://127.0.0.1:4175/api/v1/admin/boards", {
    data: {
      id: "session-board",
      board_type: "session-test",
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
  });
  const response = await request.post("http://127.0.0.1:4175/api/v1/sessions", {
    data: {
      board_type: "session-test",
      board_id: "session-board",
      client_name: "browser-test",
    },
  });
  expect(response.ok()).toBeTruthy();
  await expect(
    page.getByRole("cell", { name: "browser-test", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "强制释放" }).click();
  await page.getByRole("button", { name: "确认", exact: true }).click();
  await expect(
    page.getByRole("cell", { name: "browser-test", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("link", { name: "系统设置", exact: true }).click();
  await page.getByRole("tab", { name: "TFTP 服务", exact: true }).click();
  await expect(
    page.getByRole("tabpanel", { name: "TFTP 服务" }).getByLabel("Provider"),
  ).toHaveValue("builtin");
  await page
    .getByRole("tabpanel", { name: "TFTP 服务" })
    .getByRole("button", { name: "保存配置", exact: true })
    .click();
  await expect(
    page.getByText("TFTP 配置已保存", { exact: true }),
  ).toBeVisible();
  await page.getByRole("tab", { name: "网络与上传", exact: true }).click();
  await expect(page.getByLabel("网络接口", { exact: true })).toHaveValue("lo");
  await page.getByLabel("Session 文件上传上限（MiB）").fill("128");
  await page
    .getByRole("tabpanel", { name: "网络与上传" })
    .getByRole("button", { name: "保存配置", exact: true })
    .click();
  await expect(
    page.getByText("Server 配置已保存", { exact: true }),
  ).toBeVisible();
  await page.getByLabel("Session 文件上传上限（MiB）").fill("256");
  await page.getByRole("tab", { name: "TFTP 服务", exact: true }).click();
  await page.getByRole("tab", { name: "网络与上传", exact: true }).click();
  await expect(page.getByLabel("Session 文件上传上限（MiB）")).toHaveValue(
    "256",
  );
  await page
    .getByRole("tabpanel", { name: "网络与上传" })
    .getByRole("button", { name: "取消", exact: true })
    .click();
  await expect(page.getByLabel("Session 文件上传上限（MiB）")).toHaveValue(
    "128",
  );
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByRole("button", { name: "切换导航" })).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBeTruthy();
  await page.screenshot({
    path: "/tmp/ostool-admin-screenshots/mobile.png",
    fullPage: true,
    animations: "disabled",
  });
  expect(consoleErrors).toEqual([]);
});

test("local edition exposes the existing workflow without any account routes", async ({
  page,
  request,
}) => {
  await page.goto("/admin/overview");
  await page.getByRole("button", { name: "账号菜单", exact: true }).click();
  await expect(page.getByText("本地版 · 免登录")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("link", { name: "用户列表", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("link", { name: "租赁列表", exact: true }),
  ).toHaveCount(0);
  for (const path of [
    "/api/v1/auth/me",
    "/api/v1/admin/users",
    "/api/v1/admin/rentals",
  ]) {
    expect((await request.get(`http://127.0.0.1:4175${path}`)).status()).toBe(
      404,
    );
  }
  await page.goto("http://127.0.0.1:4175/login");
  await expect(page).toHaveURL("http://127.0.0.1:4175/");
  await expect(
    page.getByRole("link", { name: "登录", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("link", { name: "免费注册", exact: true }),
  ).toHaveCount(0);
});

test("viewport panels keep toolbar and footer fixed while tables/forms scroll", async ({
  page,
  request,
}) => {
  for (let i = 0; i < 27; i++) {
    const id = `viewport-${String(i).padStart(2, "0")}`;
    expect(
      (
        await request.post("http://127.0.0.1:4175/api/v1/admin/boards", {
          data: {
            id,
            board_type: "viewport-test",
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
    ).toBeTruthy();
  }
  await page.setViewportSize({ width: 1360, height: 800 });
  await page.goto("/admin/boards");
  await page.getByLabel("搜索开发板", { exact: true }).fill("viewport-");
  const panel = page.locator(".page-panel:visible");
  const scroll = panel.locator(".panel-scroll");
  await expect(panel.getByText("第 1 / 2 页")).toBeVisible();
  const before = await panel.evaluate((e) => ({
    top: e.querySelector(".panel-toolbar")!.getBoundingClientRect().top,
    bottom: e.querySelector(".panel-footer")!.getBoundingClientRect().bottom,
  }));
  await scroll.evaluate((e) => {
    e.scrollTop = e.scrollHeight;
  });
  expect(await scroll.evaluate((e) => e.scrollTop)).toBeGreaterThan(0);
  const after = await panel.evaluate((e) => ({
    top: e.querySelector(".panel-toolbar")!.getBoundingClientRect().top,
    bottom: e.querySelector(".panel-footer")!.getBoundingClientRect().bottom,
  }));
  expect(after).toEqual(before);
  expect(before.bottom).toBeGreaterThan(750);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollHeight <= innerHeight + 1,
    ),
  ).toBe(true);
  await panel.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(panel.getByText("第 2 / 2 页")).toBeVisible();
  await expect(
    panel.getByRole("link", { name: "viewport-26", exact: true }),
  ).toBeVisible();
  await page.getByLabel("搜索开发板", { exact: true }).focus();
  await request.put("http://127.0.0.1:4175/api/v1/admin/boards/viewport-26", {
    data: {
      id: "viewport-26",
      board_type: "viewport-test",
      tags: [],
      disabled: false,
      serial: null,
      notes: "live update",
      network_identity: null,
      boot: { kind: "pxe", notes: null },
      power_management: {
        kind: "custom",
        power_on_cmd: "true",
        power_off_cmd: "true",
      },
    },
  });
  await expect(panel.getByText("第 2 / 2 页")).toBeVisible();
  await expect(page.getByLabel("搜索开发板", { exact: true })).toBeFocused();
  await page.screenshot({ path: "/tmp/ostool-fixed-list.png", fullPage: true });
  await page
    .getByLabel("搜索开发板", { exact: true })
    .fill("no-matching-board");
  await expect(panel.getByText("暂无数据", { exact: true })).toBeVisible();
  await expect(panel.getByText("第 1 / 1 页")).toBeVisible();
  expect(
    (await panel.locator(".panel-footer").boundingBox())!.y,
  ).toBeGreaterThan(700);
  await panel.getByRole("link", { name: "新建开发板", exact: true }).click();
  await expect(page).toHaveURL(/boards\/new$/);
  await page
    .getByRole("textbox", { name: "板型", exact: true })
    .fill("viewport-test");
  await page.getByLabel("ID", { exact: true }).fill("viewport-form");
  await page.getByLabel("上电命令", { exact: true }).fill("true");
  await page.getByLabel("下电命令", { exact: true }).fill("true");
  await page.getByLabel("启动方式", { exact: true }).selectOption("pxe");
  const save = panel.getByRole("button", { name: "保存开发板", exact: true });
  await expect(save).toBeVisible();
  const buttonY = (await save.boundingBox())!.y;
  await panel.locator(".panel-scroll").evaluate((e) => {
    e.scrollTop = e.scrollHeight;
  });
  expect((await save.boundingBox())!.y).toBe(buttonY);
  await save.click();
  await expect(page).toHaveURL(/boards\/viewport-form$/);
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(save).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollHeight <= innerHeight + 1,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "/tmp/ostool-fixed-editor-mobile.png",
    fullPage: true,
  });
});

test("titlebar shares the logo theme and provides notifications, theme and workspace menus", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/admin/boards");
  await expect(
    page.getByRole("heading", { name: "开发板列表", exact: true }),
  ).toBeVisible();
  const layout = await page.evaluate(() => {
    const header = document.querySelector(".admin-topbar")!,
      logo = document.querySelector(".sidebar .brand")!;
    return {
      headerBackground: getComputedStyle(header).backgroundColor,
      logoBackground: getComputedStyle(logo).backgroundColor,
      headerHeight: header.getBoundingClientRect().height,
      logoHeight: logo.getBoundingClientRect().height,
      actions: Array.from(
        document.querySelectorAll(".admin-top-actions > button"),
      ).map((e) => e.getAttribute("aria-label")),
    };
  });
  expect(layout.headerBackground).toBe(layout.logoBackground);
  expect(layout.headerHeight).toBe(layout.logoHeight);
  expect(layout.actions).toEqual([
    expect.stringMatching(/^消息，/),
    "切换主题",
    "账号菜单",
  ]);
  const messages = page.getByRole("button", { name: /^消息，/ });
  await expect(messages).toHaveAttribute("aria-label", /1 条未读/);
  await messages.click();
  await expect(
    page.getByText("不兼容板卡配置已隔离", { exact: true }),
  ).toBeVisible();
  await expect(page.getByTestId("notification-badge")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "切换主题", exact: true }).click();
  await page.getByRole("menuitemradio", { name: "黑暗", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "切换主题", exact: true }),
  ).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByRole("menu")).not.toBeVisible();
  await expect(page.locator("html")).toHaveClass(/dark/);
  expect(
    await page.evaluate(() => localStorage.getItem("ostool.admin.theme")),
  ).toBe("dark");
  await page.screenshot({
    path: "/tmp/ostool-titlebar-dark.png",
    fullPage: true,
  });
  await page.reload();
  await expect(page.locator("html")).toHaveClass(/dark/);
  await page.getByRole("button", { name: "切换主题", exact: true }).click();
  await page
    .getByRole("menuitemradio", { name: "跟随系统", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "切换主题", exact: true }),
  ).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByRole("menu")).not.toBeVisible();
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(page.locator("html")).toHaveClass(/dark/);
  await page.getByRole("button", { name: "切换主题", exact: true }).click();
  await page.getByRole("menuitemradio", { name: "明亮", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "切换主题", exact: true }),
  ).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByRole("menu")).not.toBeVisible();
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  await page.getByRole("button", { name: "账号菜单", exact: true }).click();
  await expect(
    page.getByText("本地版 · 免登录", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "我的账号", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("menuitem", { name: "退出登录", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("menuitem", { name: "系统设置", exact: true }).click();
  await expect(page).toHaveURL(/\/admin\/settings$/);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "切换导航", exact: true }).click();
  await expect(
    page.getByRole("link", { name: "开发板列表", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "切换导航", exact: true }).click();
  await page.getByRole("button", { name: "切换主题", exact: true }).click();
  await expect(
    page.getByRole("menuitemradio", { name: "跟随系统", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        document.documentElement.scrollWidth <= innerWidth &&
        document.documentElement.scrollHeight <= innerHeight + 1,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "/tmp/ostool-titlebar-mobile.png",
    fullPage: true,
  });
});

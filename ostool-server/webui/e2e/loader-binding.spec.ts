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
  await expect(peer.getByText("离线", { exact: true })).toBeVisible({
    timeout: 15_000,
  });
  expect(
    reads.every((url) => url.endsWith("/api/v1/admin/events")),
  ).toBeTruthy();
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
    page.getByRole("heading", { name: "总览", exact: true }),
  ).toBeVisible();
  await page.getByRole("link", { name: "DTB", exact: true }).click();
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
  await page.getByRole("link", { name: "TFTP", exact: true }).click();
  await expect(page.getByLabel("Provider")).toHaveValue("builtin");
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(
    page.getByText("TFTP 配置已保存", { exact: true }),
  ).toBeVisible();
  await page.getByRole("link", { name: "Server 配置", exact: true }).click();
  await expect(page.getByLabel("网络接口", { exact: true })).toHaveValue("lo");
  await page.getByLabel("Session 文件上传上限（MiB）").fill("128");
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(
    page.getByText("Server 配置已保存", { exact: true }),
  ).toBeVisible();
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

import { expect, test } from "playwright/test";

test("dashboard, durable audit filters/export, discovery toolbar and shared theme styles", async ({
  page,
  request,
}) => {
  test.setTimeout(90_000);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.goto("/admin/overview");
  await expect(
    page.getByRole("heading", { name: "服务运行状态" }),
  ).toBeVisible();
  await expect(
    page.getByRole("status").filter({ hasText: "实时已连接" }),
  ).toBeVisible();
  await expect(page.locator(".sidebar .connection")).toHaveCount(0);
  await expect(page.locator(".dashboard-footer")).not.toContainText(
    "正在采集",
    { timeout: 15_000 },
  );
  await page.screenshot({ path: "/tmp/ostool-dashboard-runtime.png" });
  await page.getByRole("link", { name: "后台任务", exact: true }).click();
  await expect(
    page.getByRole("cell", { name: /服务器运行采样/ }),
  ).toBeVisible();
  await expect(page.getByRole("cell", { name: /会话租约清理/ })).toBeVisible();
  await page.getByRole("link", { name: "运营状态", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "开发板状态分布" }),
  ).toBeVisible();
  await expect(page.getByText("用户总数", { exact: true })).toHaveCount(0);
  await page.getByRole("link", { name: "设备列表", exact: true }).click();
  await expect(page.getByRole("tablist")).toHaveCount(0);
  await expect(page.getByRole("button", { name: /发现设备/ })).toBeVisible();

  const id = `audit-browser-${Date.now()}`;
  const response = await request.post(
    "http://127.0.0.1:4175/api/v1/admin/boards",
    {
      data: {
        id,
        board_type: "audit-test",
        tags: [],
        disabled: false,
        serial: null,
        notes: "private note must not enter the audit",
        network_identity: null,
        boot: { kind: "pxe", notes: null },
        power_management: {
          kind: "custom",
          power_on_cmd: "true",
          power_off_cmd: "true",
        },
      },
    },
  );
  expect(response.status()).toBe(201);
  await expect(page.getByRole("link", { name: id, exact: true })).toBeVisible();
  const fonts = await page.evaluate(() => {
    const nav = document.querySelector('.sidebar a[href="/admin/boards"]')!;
    const cell = document.querySelector('[data-slot="table-cell"]')!;
    return [nav, cell].map((element) => {
      const style = getComputedStyle(element);
      return { size: style.fontSize, family: style.fontFamily };
    });
  });
  expect(fonts[0]).toEqual(fonts[1]);
  expect(fonts[0].size).toBe("16px");
  const tableRow = page
    .getByRole("row")
    .filter({ has: page.getByRole("link", { name: id, exact: true }) });
  const listStyles = await tableRow.evaluate((row) => {
    const style = (element: Element) => {
      const css = getComputedStyle(element);
      return {
        size: css.fontSize,
        family: css.fontFamily,
        weight: css.fontWeight,
      };
    };
    return {
      name: style(row.querySelector(".board-name")!),
      connection: style(row.querySelectorAll("td")[2]),
      secondary: style(row.querySelector("small")!),
    };
  });
  await page.getByRole("button", { name: "切换列表布局", exact: true }).click();
  await page
    .getByRole("menuitemradio", { name: "卡片视图", exact: true })
    .click();
  const card = page
    .getByRole("listitem")
    .filter({ has: page.getByRole("link", { name: id, exact: true }) });
  const cardStyles = await card.evaluate((card) => {
    const style = (element: Element) => {
      const css = getComputedStyle(element);
      return {
        size: css.fontSize,
        family: css.fontFamily,
        weight: css.fontWeight,
      };
    };
    return {
      name: style(card.querySelector(".board-name")!),
      connection: style(card.querySelectorAll("dd")[1]),
      secondary: style(card.querySelector("small")!),
    };
  });
  expect(cardStyles).toEqual(listStyles);
  await page.getByRole("button", { name: "切换列表布局", exact: true }).click();
  await page
    .getByRole("menuitemradio", { name: "列表视图", exact: true })
    .click();

  const reads: string[] = [];
  page.on("request", (r) => {
    if (r.method() === "GET" && new URL(r.url()).pathname.includes("/audits"))
      reads.push(r.url());
  });
  await page.getByRole("link", { name: "审计日志", exact: true }).click();
  const search = page.getByLabel("搜索审计日志", { exact: true });
  await search.fill(id);
  await search.focus();
  const row = page.getByRole("row").filter({ hasText: id });
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("新增开发板");
  await expect(row).toContainText("本地操作者");
  await expect(search).toBeFocused();
  expect(reads).toEqual([]);
  await row.getByRole("button", { name: "详情", exact: true }).click();
  await expect(page.getByRole("dialog")).toContainText(id);
  await expect(page.getByRole("dialog")).not.toContainText(
    "private note must not enter the audit",
  );
  await page.getByRole("button", { name: "关闭", exact: true }).click();
  const downloadEvent = page.waitForEvent("download");
  await page.getByRole("button", { name: "导出 CSV", exact: true }).click();
  const download = await downloadEvent;
  expect(download.suggestedFilename()).toBe("ostool-audit.csv");
  expect(await download.failure()).toBeNull();
  await page.getByRole("button", { name: "更多筛选", exact: true }).click();
  await page.getByLabel("来源 IP", { exact: true }).fill("invalid");
  await expect(
    page.getByText("请输入有效的 IPv4 或 IPv6 地址", { exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "导出 CSV" })).toBeDisabled();
  await page.getByLabel("来源 IP", { exact: true }).fill("");
  await page.screenshot({ path: "/tmp/ostool-audit-list.png" });

  await page.getByRole("button", { name: "切换主题", exact: true }).click();
  const adminStyle = await page
    .getByRole("menuitemradio", { name: "黑暗", exact: true })
    .evaluate((element) => ({
      font: getComputedStyle(element).fontSize,
      icon: parseFloat(
        getComputedStyle(element.querySelector("svg.theme-option-icon")!).width,
      ),
    }));
  await page.keyboard.press("Escape");
  await page.goto("http://127.0.0.1:4175/");
  await page.getByRole("button", { name: "切换网站主题", exact: true }).click();
  const siteStyle = await page
    .getByRole("menuitemradio", { name: "黑暗", exact: true })
    .evaluate((element) => ({
      font: getComputedStyle(element).fontSize,
      icon: parseFloat(
        getComputedStyle(element.querySelector("svg.theme-option-icon")!).width,
      ),
    }));
  expect(siteStyle).toEqual(adminStyle);
  expect(siteStyle).toEqual({ font: "16px", icon: 16 });
  await page.getByRole("menuitemradio", { name: "黑暗", exact: true }).click();
  await expect(page.locator("html")).toHaveClass(/dark/);
  await page.getByRole("button", { name: "切换网站主题", exact: true }).click();
  await page.screenshot({ path: "/tmp/ostool-site-theme.png" });
  expect(
    (
      await request.delete(`http://127.0.0.1:4175/api/v1/admin/boards/${id}`)
    ).ok(),
  ).toBeTruthy();
  expect(errors).toEqual([]);
});

test("runtime health follows a lost event stream and recovers without a page banner", async ({
  page,
}) => {
  await page.goto("/admin/overview");
  const summary = page.getByRole("region", { name: "运行状态汇总" });
  await expect(summary.getByRole("status")).toHaveText("服务运行正常", {
    timeout: 15_000,
  });
  try {
    await page.route("**/api/v1/admin/events**", (route) =>
      route.abort("connectionfailed"),
    );
    // Loopback SSE connections can survive Chromium's offline emulation. Interrupt
    // the subscription itself, then block actual reconnection requests instead.
    await page.evaluate(async () => {
      const eventModule = "/src/api/events.ts";
      const { admin } = await import(eventModule);
      admin.restart();
    });
    await expect(summary.getByRole("status")).toHaveText("连接中断，正在重连");
    await expect(summary.getByRole("status")).toHaveClass(/warning/);
    await expect(page.locator(".admin-notices [data-slot=alert]")).toHaveCount(
      0,
    );
    await expect(page.getByText("待重新确认", { exact: true })).toHaveCount(2);
    await expect(
      page.getByText("连接中断，自动重连中", { exact: true }),
    ).toBeVisible();
  } finally {
    await page.unroute("**/api/v1/admin/events**");
  }
  await expect(summary.getByRole("status")).toHaveText("服务运行正常", {
    timeout: 15_000,
  });
  await expect(page.getByText("待重新确认", { exact: true })).toHaveCount(0);
  await expect(
    page.getByRole("status").filter({ hasText: "实时已连接" }),
  ).toBeVisible();
});

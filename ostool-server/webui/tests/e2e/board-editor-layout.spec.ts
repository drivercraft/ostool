import { expect, test } from "../fixtures/local";

test("new and existing device forms keep their layout while configuration modes change", async ({
  page,
  request,
}) => {
  const board = "fixed-layout-board";
  const created = await request.post(
    "http://127.0.0.1:4175/api/v1/admin/boards",
    {
      data: {
        id: board,
        board_type: "layout-test",
        disabled: true,
        tags: [],
        serial: null,
        network_identity: null,
        boot: { kind: "pxe" },
        power_management: {
          kind: "custom",
          power_on_cmd: "true",
          power_off_cmd: "true",
        },
      },
    },
  );
  expect(created.status()).toBe(201);
  for (const route of ["/qcl/boards/new", `/qcl/boards/${board}`]) {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto(route);
    const form = page.locator(".board-editor-form");
    await expect(page.getByLabel("板型", { exact: true })).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    const layout = () =>
      form.evaluate((element) => {
        const top = element.getBoundingClientRect().top;
        return {
          height: Math.round(element.getBoundingClientRect().height),
          fields: Array.from(
            element.querySelectorAll(
              "input:not([type=checkbox]), select, textarea",
            ),
            (field) => {
              const bounds = field.getBoundingClientRect();
              return [
                field.id,
                Math.round(bounds.top - top),
                Math.round(bounds.width),
              ];
            },
          ),
        };
      });
    const initial = await layout();
    for (const kind of ["httpboot", "pxe", "uboot"]) {
      await page.getByLabel("启动方式", { exact: true }).selectOption(kind);
      await expect(page.getByLabel("MAC 地址", { exact: true })).toBeEnabled({
        enabled: kind !== "uboot",
      });
      await expect(page.getByLabel("PXE 备注", { exact: true })).toBeEnabled({
        enabled: kind === "pxe",
      });
      expect(await layout()).toEqual(initial);
    }
    await page.getByLabel("启用串口", { exact: true }).check();
    await page.getByLabel("波特率", { exact: true }).fill("57600");
    await page.getByLabel("启用串口", { exact: true }).uncheck();
    await expect(page.getByLabel("波特率", { exact: true })).toBeDisabled();
    await expect(page.getByLabel("波特率", { exact: true })).toHaveValue(
      "57600",
    );
    await page.getByLabel("启用 TFTP", { exact: true }).check();
    await page
      .getByLabel("网络模式", { exact: true })
      .selectOption("static_ip");
    await page.getByLabel("开发板 IP", { exact: true }).fill("192.0.2.10");
    await page.getByLabel("网络模式", { exact: true }).selectOption("dhcp");
    await expect(page.getByLabel("开发板 IP", { exact: true })).toBeDisabled();
    await expect(page.getByLabel("开发板 IP", { exact: true })).toHaveValue(
      "192.0.2.10",
    );
    for (const kind of ["zhongsheng_relay", "qemu", "custom"]) {
      await page.getByLabel("电源模块", { exact: true }).selectOption(kind);
      expect(await layout()).toEqual(initial);
    }
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      expect(
        await form.evaluate((el) => el.scrollWidth <= el.clientWidth),
      ).toBe(true);
      await page.locator(".panel-scroll").evaluate((el) => {
        el.scrollTop = 0;
      });
      await page.screenshot({
        path: `/tmp/ostool-board-${route.endsWith("new") ? "new" : "edit"}-${width}.png`,
        fullPage: true,
      });
    }
  }
  await request.delete("http://127.0.0.1:4175/api/v1/admin/boards/" + board);
});

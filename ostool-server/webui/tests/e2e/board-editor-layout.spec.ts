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
    const tags = page.getByLabel("标签", { exact: true });
    const notes = page.getByLabel("备注", { exact: true });
    await tags.fill("C 区实验室, ARM64, 驱动开发");
    await notes.fill(
      "型号：Ampere Altra 研发节点\n位置：C 区 / 机架 03 / 槽位 01\n负责人：周航\n用途：调度器性能与时延评估",
    );
    const metadataHeights = async () => {
      const [tagBox, noteBox] = await Promise.all([
        tags.boundingBox(),
        notes.boundingBox(),
      ]);
      return Math.abs(tagBox!.height - noteBox!.height);
    };
    await expect.poll(metadataHeights).toBeLessThan(1);
    const powerSection = form.locator(".form-section").filter({
      has: page.getByRole("heading", { name: "电源管理", exact: true }),
    });
    await expect(powerSection.locator(":scope > .form-fields")).toHaveAttribute(
      "data-columns",
      "3",
    );
    for (const direction of ["上电", "下电"]) {
      const field = powerSection.getByRole("group", {
        name: `${direction}测试`,
        exact: true,
      });
      await expect(
        field.getByLabel(`${direction}命令`, { exact: true }),
      ).toBeVisible();
      await expect(
        field.getByRole("button", { name: `${direction}测试`, exact: true }),
      ).toBeVisible();
    }
    const desktop = await powerSection.evaluate((section) => {
      const module = section.querySelector("select")!.getBoundingClientRect();
      return Array.from(
        section.querySelectorAll(".board-inline-field"),
        (field) => {
          const input = field.querySelector("input")!.getBoundingClientRect();
          const button = field
            .querySelector("button:not(.field-help-trigger)")!
            .getBoundingClientRect();
          return {
            top: input.top - module.top,
            buttonTop: button.top - input.top,
            height: input.height,
            buttonHeight: button.height,
            moduleHeight: module.height,
          };
        },
      );
    });
    for (const field of desktop) {
      expect(field.top).toBeGreaterThan(42);
      expect(Math.abs(field.buttonTop)).toBeLessThan(1);
      expect(field.height).toBe(field.buttonHeight);
      expect(field.height).toBe(field.moduleHeight);
    }
    for (const labels of [
      ["电源模块", "虚拟设备", "继电器"],
      ["标签", "备注"],
      ["网络模式", "开发板 IP", "服务器 IP"],
    ]) {
      const boxes = await Promise.all(
        labels.map((label) =>
          page.getByLabel(label, { exact: true }).boundingBox(),
        ),
      );
      for (const box of boxes)
        expect(Math.abs(box!.y - boxes[0]!.y)).toBeLessThan(1);
    }
    await expect(form.locator(".form-fields .hint")).toHaveCount(0);
    await expect(powerSection.locator(":scope > div > .hint")).toBeVisible();
    await expect(
      page.getByRole("button", { name: "电源管理说明", exact: true }),
    ).toHaveCount(0);
    await page
      .getByRole("button", { name: "上电命令说明", exact: true })
      .click();
    await expect(
      page.getByRole("note", { name: "上电命令说明", exact: true }),
    ).toBeVisible();
    await page.keyboard.press("Escape");
    const controlHeights = await form.evaluate((element) =>
      Array.from(
        element.querySelectorAll(
          "input:not([type=checkbox]), select, button:not([role=checkbox]):not([role=switch]):not(.field-help-trigger)",
        ),
        (control) => control.getBoundingClientRect().height,
      ),
    );
    for (const height of controlHeights) expect(height).toBe(controlHeights[0]);
    const serialSwitch = page.getByRole("switch", {
      name: "启用串口",
      exact: true,
    });
    const switchRow = await serialSwitch.locator("..").boundingBox();
    const serialSelect = await page
      .getByLabel("稳定串口", { exact: true })
      .boundingBox();
    expect(Math.abs(switchRow!.y - serialSelect!.y)).toBeLessThan(1);
    expect(switchRow!.height).toBe(serialSelect!.height);
    const mac = await page
      .getByLabel("MAC 地址", { exact: true })
      .boundingBox();
    const discover = await page
      .getByRole("button", { name: /^发现 \d+ 台设备$/ })
      .boundingBox();
    expect(Math.abs(mac!.y - discover!.y)).toBeLessThan(1);
    await powerSection.screenshot({
      path: `/tmp/ostool-board-power-${route.endsWith("new") ? "new" : "edit"}-1440.png`,
    });
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
      await expect.poll(metadataHeights).toBeLessThan(1);
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

test("OTA image actions align and show upload and dispatch progress", async ({
  page,
  request,
}) => {
  const board = "ota-alignment-board";
  const endpoint = "http://127.0.0.1:4175/api/v1/admin/boards/" + board;
  const created = await request.post(
    "http://127.0.0.1:4175/api/v1/admin/boards",
    {
      data: {
        id: board,
        board_type: "layout-test",
        disabled: true,
        tags: [],
        serial: null,
        network_identity: { mac_address: "02:00:00:00:00:75" },
        boot: { kind: "httpboot" },
        power_management: {
          kind: "custom",
          power_on_cmd: "true",
          power_off_cmd: "true",
        },
      },
    },
  );
  expect(created.status()).toBe(201);
  try {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto(`/qcl/boards/${board}`);
    const section = page.locator(".form-section").filter({
      has: page.getByRole("heading", { name: "axloader OTA", exact: true }),
    });
    await expect(
      section.getByLabel("待下发镜像", { exact: true }),
    ).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    const [upload, select, remove, dispatch] = await Promise.all([
      section
        .getByRole("button", { name: "上传镜像", exact: true })
        .boundingBox(),
      section.getByLabel("待下发镜像", { exact: true }).boundingBox(),
      section
        .getByRole("button", { name: "删除镜像", exact: true })
        .boundingBox(),
      section
        .getByRole("button", { name: "下发升级", exact: true })
        .boundingBox(),
    ]);
    for (const button of [upload!, remove!, dispatch!]) {
      expect(Math.abs(button.y - select!.y)).toBeLessThan(1);
      expect(button.height).toBe(select!.height);
    }
    expect(select!.x).toBeLessThan(upload!.x);
    expect(upload!.x).toBeLessThan(remove!.x);
    expect(remove!.x).toBeLessThan(dispatch!.x);
    const [address, credential] = await Promise.all([
      page.getByLabel("设备 HTTP / HTTPS 地址", { exact: true }).boundingBox(),
      page.getByLabel("设备独立凭据", { exact: true }).boundingBox(),
    ]);
    expect(Math.abs(address!.y - credential!.y)).toBeLessThan(1);
    expect(Math.abs(address!.width - credential!.width)).toBeLessThan(1);
    await section.screenshot({ path: "/tmp/ostool-board-ota-1440.png" });
    let releaseUpload: (() => void) | undefined;
    let uploadedSha = "";
    await page.route("**/api/v1/admin/loader-images", async (route) => {
      const response = await route.fetch();
      uploadedSha = (await response.json()).sha256;
      await new Promise<void>((resolve) => {
        releaseUpload = resolve;
      });
      await route.fulfill({ response });
    });
    await section.getByLabel("上传 EFI 镜像", { exact: true }).setInputFiles({
      name: "node-update.efi",
      mimeType: "application/octet-stream",
      buffer: Buffer.from("fixture EFI"),
    });
    await expect(
      section.getByRole("progressbar", { name: "镜像上传进度" }),
    ).toBeVisible();
    await expect.poll(() => !!releaseUpload).toBe(true);
    await section.screenshot({ path: "/tmp/ostool-ota-upload-progress.png" });
    releaseUpload!();
    await expect(section.getByLabel("待下发镜像", { exact: true })).toHaveValue(
      uploadedSha,
    );
    await expect(section.locator("option:checked")).toHaveText(
      "node-update.efi",
    );
    await expect(section.getByRole("progressbar")).toHaveCount(0);
    await page.route(
      `**/api/v1/admin/boards/${board}/loader-updates`,
      async (route) => {
        expect(route.request().postDataJSON().image_sha256).toBe(uploadedSha);
        await route.fulfill({
          json: {
            board_id: board,
            mac_address: "02:00:00:00:00:75",
            update_id: "progress-fixture",
            image: {
              sha256: uploadedSha,
              size: 11,
              version: "node-update.efi",
            },
            phase: "queued",
            error: null,
            delivery_attempts: 0,
          },
        });
      },
    );
    await section
      .getByRole("button", { name: "下发升级", exact: true })
      .click();
    await expect(
      section.getByRole("progressbar", { name: "镜像下发进度" }),
    ).toBeVisible();
    await expect(section.getByRole("progressbar")).not.toHaveAttribute(
      "aria-valuenow",
    );
    await section.screenshot({ path: "/tmp/ostool-ota-dispatch-progress.png" });
    await page.setViewportSize({ width: 390, height: 1000 });
    expect(
      await section.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
  } finally {
    await request.delete(endpoint);
  }
});

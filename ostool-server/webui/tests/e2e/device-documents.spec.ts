import { test, expect } from "playwright/test";
import { apiLogin } from "../fixtures/security";

test("device documents publish to product details and the rental workspace with the common website footer", async ({
  browser,
}) => {
  test.setTimeout(90_000);
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires disposable public fixture");
  const context = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
    storageState: { cookies: [], origins: [] },
  });
  try {
    const { accessToken } = await apiLogin(
      context.request,
      base!,
      process.env.OSTOOL_E2E_ADMIN_EMAIL!,
      process.env.OSTOOL_E2E_ADMIN_PASSWORD!,
    );
    const id = `document-board-${Date.now()}`;
    const created = await context.request.post(base + "/api/v1/admin/boards", {
      headers: { authorization: `Bearer ${accessToken}`, origin: base! },
      data: {
        id,
        board_type: "arm64-lab",
        tags: ["kernel", "serial"],
        boot: { kind: "uboot", use_tftp: false },
        power_management: {
          kind: "custom",
          power_on_cmd: "true # PRIVATE POWER",
          power_off_cmd: "true",
        },
        notes: "PRIVATE OPERATIONS",
      },
    });
    expect(created.status()).toBe(201);
    const page = await context.newPage();
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto(`${base}/admin/device-documents`);
    await page.getByLabel("搜索设备文档").fill(id);
    const documentToolbar = page.getByRole("toolbar", {
      name: "列表快捷工具栏",
    });
    const toolbarBox = await documentToolbar.boundingBox();
    const searchBox = await page.getByLabel("搜索设备文档").boundingBox();
    expect(searchBox!.x).toBeGreaterThan(toolbarBox!.x + toolbarBox!.width / 2);
    await page.screenshot({
      path: "/tmp/ostool-device-document-list.png",
      fullPage: true,
    });

    await page.getByRole("link", { name: "编辑文档", exact: true }).click();
    await page
      .getByLabel("公开标题", { exact: true })
      .fill("Explorer development board");
    await page
      .getByLabel("设备简介", { exact: true })
      .fill("A workspace for your next kernel.");
    await page
      .getByLabel("详细介绍", { exact: true })
      .fill("Device documentation authored in the management console.");
    await page.getByRole("button", { name: "添加亮点", exact: true }).click();
    await page
      .getByLabel("亮点 1", { exact: true })
      .fill("UART development workflow");
    await page.getByRole("button", { name: "添加参数", exact: true }).click();
    await page.getByLabel("参数名称 1", { exact: true }).fill("Memory");
    await page.getByLabel("参数值 1", { exact: true }).fill("8 GiB");
    const editorLayout = await page.evaluate(() => {
      const box = (selector: string) =>
        document.querySelector(selector)!.getBoundingClientRect().toJSON();
      return {
        form: box(".device-document-form"),
        description: box(".device-document-description"),
      };
    });
    expect(editorLayout.description.height).toBeGreaterThanOrEqual(240);
    expect(editorLayout.description.width).toBeGreaterThan(
      editorLayout.form.width * 0.85,
    );
    const parameters = page
      .getByRole("heading", { name: "公开参数", exact: true })
      .locator("..")
      .locator("..");
    const nameBox = await parameters
      .getByLabel("参数名称 1", { exact: true })
      .boundingBox();
    const valueBox = await parameters
      .getByLabel("参数值 1", { exact: true })
      .boundingBox();
    expect(Math.abs(nameBox!.y - valueBox!.y)).toBeLessThan(2);
    expect(nameBox!.x + nameBox!.width).toBeLessThan(valueBox!.x);
    await page
      .locator(".device-document-editor .panel-scroll")
      .evaluate((el) => {
        el.scrollTop = 0;
      });
    // Compare with the existing new-board editor, including wide screens that
    // exposed the old centered max-width and nested card gutters.
    const referencePage = await context.newPage();
    await referencePage.goto(`${base}/admin/boards/new`);
    await expect(
      referencePage.getByLabel("板型", { exact: true }),
    ).toBeVisible();
    const editorBounds = async (target: typeof page) =>
      target.evaluate(() => {
        const box = (selector: string) =>
          document.querySelector(selector)!.getBoundingClientRect().toJSON();
        const scroll = document.querySelector(".page-panel > .panel-scroll")!;
        scroll.scrollTop = 0;
        return {
          panel: box(".page-panel"),
          form: box(".panel-scroll > form"),
          section: box(".panel-scroll > form > .form-section"),
          footer: box(".page-panel > .panel-footer"),
        };
      });
    for (const width of [1440, 1920, 2560, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      await referencePage.setViewportSize({ width, height: 1000 });
      const actual = await editorBounds(page),
        expected = await editorBounds(referencePage);
      for (const key of ["panel", "form", "section", "footer"] as const) {
        expect(
          Math.abs(actual[key].left - expected[key].left),
          `${width}px ${key} left`,
        ).toBeLessThan(1);
        expect(
          Math.abs(actual[key].right - expected[key].right),
          `${width}px ${key} right`,
        ).toBeLessThan(1);
      }
      expect(
        Math.abs(actual.section.top - expected.section.top),
        `${width}px content top`,
      ).toBeLessThan(1);
    }
    await referencePage.close();
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.screenshot({
      path: "/tmp/ostool-device-document-editor.png",
      fullPage: true,
    });
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await expect(
      page.getByRole("button", { name: "保存并发布", exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: "/tmp/ostool-device-document-editor-mobile.png",
      fullPage: true,
    });
    await page.setViewportSize({ width: 1440, height: 1000 });

    await page.getByRole("button", { name: "保存并发布", exact: true }).click();
    await expect(
      page.getByText("设备文档已发布", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("文档已被其他管理员更新，当前输入已保留。"),
    ).toHaveCount(0);
    await page.reload();
    await expect(page.getByLabel("公开标题", { exact: true })).toHaveValue(
      "Explorer development board",
    );
    // A remote update must flag a conflict without replacing the editor's input.
    await page
      .getByLabel("设备简介", { exact: true })
      .fill("Keep this unsaved draft");
    const current = await (
      await context.request.get(`${base}/api/v1/admin/device-documents/${id}`)
    ).json();
    const revised = await context.request.put(
      `${base}/api/v1/admin/device-documents/${id}`,
      {
        headers: { origin: base! },
        data: {
          revision: current.revision,
          document: {
            ...current.document,
            description: "Published through the device documentation page.",
          },
        },
      },
    );
    expect(revised.status()).toBe(200);
    await expect(
      page.getByText("文档已被其他管理员更新，当前输入已保留。"),
    ).toBeVisible();
    await expect(page.getByLabel("设备简介", { exact: true })).toHaveValue(
      "Keep this unsaved draft",
    );
    const email = `doc-user-${Date.now()}@test.local`;
    const member = await context.request.post(`${base}/api/v1/admin/users`, {
      headers: { origin: base! },
      data: {
        username: `doc_user_${Date.now()}`,
        email,
        password: "device-doc-password-123",
        display_name: "Doc member",
        role_id: "member",
        disabled: false,
      },
    });
    expect(member.status(), await member.text()).toBe(201);
    await apiLogin(
      context.request,
      base!,
      email,
      "device-doc-password-123",
      false,
    );
    await page.goto(base + "/devices");
    await page.getByLabel("搜索设备").fill(id);
    const card = page.locator(".device-directory-card").filter({
      has: page.getByRole("heading", {
        name: "Explorer development board",
        exact: true,
      }),
    });
    const details = card.getByRole("link", { name: "查看详情", exact: true }),
      apply = card.getByRole("link", { name: "申请租赁", exact: true });
    const db = await details.boundingBox(),
      ab = await apply.boundingBox();
    expect(Math.abs(db!.y - ab!.y)).toBeLessThan(2);
    expect(db!.x + db!.width).toBeLessThanOrEqual(ab!.x);
    expect((await card.boundingBox())!.width).toBeGreaterThan(390);
    await details.click();
    await expect(page).toHaveURL(`${base}/devices/${id}`);
    await expect(
      page.getByRole("heading", {
        level: 1,
        name: "Explorer development board",
      }),
    ).toBeVisible();
    await expect(page.getByText("8 GiB", { exact: true })).toBeVisible();
    await expect(
      page.getByText("Published through the device documentation page."),
    ).toBeVisible();
    expect(await page.locator("main").textContent()).not.toContain("PRIVATE");
    await page.reload();
    await expect(page.getByRole("heading", { level: 1 })).toHaveText(
      "Explorer development board",
    );
    const portalReference = await context.newPage();
    for (const width of [1440, 1920, 2560, 900, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      await portalReference.setViewportSize({ width, height: 1000 });
      await portalReference.goto(base + "/");
      await expect(
        portalReference.locator(".public-hero-section"),
      ).toBeVisible();
      const homeBounds = await portalReference
        .locator(".public-hero-section")
        .boundingBox();
      await portalReference.goto(base + "/devices");
      await expect(portalReference.locator(".device-directory")).toBeVisible();
      const listBounds = await portalReference
        .locator(".devices-page")
        .boundingBox();
      const detailBounds = await page
        .locator(".device-detail-page")
        .boundingBox();
      for (const [name, reference] of [
        ["home", homeBounds],
        ["device list", listBounds],
      ] as const) {
        expect(
          Math.abs(detailBounds!.x - reference!.x),
          `${width}px detail main vs ${name} left`,
        ).toBeLessThan(1);
        expect(
          Math.abs(
            detailBounds!.x +
              detailBounds!.width -
              reference!.x -
              reference!.width,
          ),
          `${width}px detail main vs ${name} right`,
        ).toBeLessThan(1);
      }
      const alignment = await page.evaluate(() => {
        const header = document
          .querySelector(".site-header-inner")!
          .getBoundingClientRect();
        return [
          ".device-detail-nav",
          ".device-detail-hero",
          ".device-detail-introduction",
          ".device-detail-specifications",
        ].map((selector) => {
          const content = document
            .querySelector(selector)!
            .getBoundingClientRect();
          return {
            selector,
            left: content.left - header.left,
            right: content.right - header.right,
          };
        });
      });
      for (const section of alignment) {
        expect(
          Math.abs(section.left),
          `${width}px ${section.selector} left`,
        ).toBeLessThan(1);
        expect(
          Math.abs(section.right),
          `${width}px ${section.selector} right`,
        ).toBeLessThan(1);
      }
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
    }
    await portalReference.close();
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.screenshot({
      path: "/tmp/ostool-device-product-detail.png",
      fullPage: true,
    });
    await page
      .getByRole("navigation", { name: "设备详情导航" })
      .getByRole("link", { name: "申请租赁", exact: true })
      .click();
    await expect(page.getByLabel("申请开发板", { exact: true })).toHaveValue(
      id,
    );
    await expect(
      page
        .getByRole("complementary", { name: "所选设备信息" })
        .getByText("8 GiB", { exact: true }),
    ).toBeVisible();
    const geometry = await page.evaluate(() => ({
      viewport: innerHeight,
      application: document
        .querySelector(".rental-request-page")!
        .getBoundingClientRect()
        .toJSON(),
      header: document
        .querySelector(".site-header")!
        .getBoundingClientRect()
        .toJSON(),
      footer: document
        .querySelector(".site-footer")!
        .getBoundingClientRect()
        .toJSON(),
      left: document
        .querySelector(".rental-device-summary")!
        .getBoundingClientRect()
        .toJSON(),
      right: document
        .querySelector(".rental-request-panel")!
        .getBoundingClientRect()
        .toJSON(),
    }));
    expect(geometry.application.height).toBeLessThanOrEqual(
      geometry.viewport - geometry.header.height + 1,
    );
    expect(geometry.footer.top).toBeGreaterThanOrEqual(
      geometry.application.bottom - 1,
    );
    await expect(page.locator(".site-header")).toHaveCount(1);
    await expect(page.locator(".site-footer")).toHaveCount(1);
    expect(geometry.left.right).toBeLessThan(geometry.right.left);
    expect(Math.abs(geometry.left.top - geometry.right.top)).toBeLessThan(2);
    const rightFrame = await page
      .locator(".rental-request-panel")
      .evaluate((el) => {
        const style = getComputedStyle(el);
        return {
          radius: style.borderRadius,
          borders: [
            style.borderTopWidth,
            style.borderRightWidth,
            style.borderBottomWidth,
            style.borderLeftWidth,
          ],
          shadow: style.boxShadow,
        };
      });
    expect(rightFrame.radius).toBe("0px");
    expect(rightFrame.borders).toEqual(["0px", "0px", "0px", "0px"]);
    expect(rightFrame.shadow).toBe("none");
    for (const selector of [".panel-toolbar", ".panel-footer"])
      expect(
        await page
          .locator(".rental-request-panel " + selector)
          .evaluate((el) => getComputedStyle(el).borderRadius),
      ).toBe("0px");

    await page.screenshot({
      path: "/tmp/ostool-device-rental-split.png",
      fullPage: true,
    });
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () =>
          document.documentElement.scrollWidth <= innerWidth &&
          document
            .querySelector(".rental-request-page")!
            .getBoundingClientRect().height <= innerHeight,
      ),
    ).toBe(true);
    await expect(
      page.getByRole("button", { name: "提交申请", exact: true }),
    ).toBeVisible();
    await page.locator(".site-footer").scrollIntoViewIfNeeded();
    await expect(page.locator(".site-footer")).toBeVisible();
    await page.evaluate(() => window.scrollTo(0, 0));
    await page.screenshot({
      path: "/tmp/ostool-device-rental-mobile.png",
      fullPage: true,
    });
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

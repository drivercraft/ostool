import { solveCaptcha, apiLogin, apiRegister } from "../fixtures/security";
import { expect, test } from "playwright/test";
test("standalone rental calendar, live conflicts and persistent card layouts", async ({
  page,
  request,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires disposable PostgreSQL public server");
  const email = process.env.OSTOOL_E2E_ADMIN_EMAIL ?? "admin@example.com",
    password = process.env.OSTOOL_E2E_ADMIN_PASSWORD ?? "admin-password-123";
  const login = await apiLogin(request, base!, email, password);
  const headers = { authorization: "Bearer " + login.accessToken };
  const me = await (
    await request.get(`${base}/api/v1/auth/me`, { headers })
  ).json();
  const boardId = `calendar-board-${Date.now()}`;
  expect(
    (
      await request.post(`${base}/api/v1/admin/boards`, {
        headers,
        data: {
          id: boardId,
          board_type: "calendar-test",
          tags: ["browser"],
          serial: null,
          notes: null,
          disabled: false,
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
  const start = new Date();
  start.setDate(start.getDate() + 3);
  start.setHours(9, 0, 0, 0);
  const end = new Date(start.getTime() + 7200000);
  expect(
    (
      await request.post(`${base}/api/v1/admin/rentals`, {
        headers,
        data: {
          board_id: boardId,
          user_id: me.id,
          starts_at: start.toISOString(),
          ends_at: end.toISOString(),
          notes: "Existing fixture booking",
        },
      })
    ).ok(),
  ).toBeTruthy();
  await page.goto(`${base}/admin/login?redirect=/admin/rentals`);
  await page.getByLabel("邮箱", { exact: true }).fill(email);
  await page.getByLabel("密码", { exact: true }).fill(password);
  await solveCaptcha(page);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(page).toHaveURL(`${base}/admin/rentals`);
  await page.getByRole("link", { name: "新增租赁", exact: true }).click();
  await expect(page).toHaveURL(`${base}/admin/rentals/new`);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("region", { name: "租赁日历" })).toBeVisible();
  await page.getByLabel("指定开发板", { exact: true }).selectOption(boardId);
  await page.getByLabel("使用用户", { exact: true }).selectOption(me.id);
  const dates = await page.evaluate(
    ([s, e]) => {
      const local = (value: number) => {
        const d = new Date(value);
        return new Date(d.getTime() - d.getTimezoneOffset() * 60000)
          .toISOString()
          .slice(0, 16);
      };
      return [local(s), local(e)];
    },
    [start.getTime() + 3600000, end.getTime() + 3600000],
  );
  await page.getByLabel("开始时间", { exact: true }).fill(dates[0]);
  await page.getByLabel("结束时间", { exact: true }).fill(dates[1]);
  await page.getByLabel("备注", { exact: true }).fill("My calendar draft");
  await expect(
    page.getByRole("button", { name: "保存租赁", exact: true }),
  ).toBeDisabled();
  await expect(page.getByText(/所选时段已有租赁/)).toBeVisible();
  const calendar = page.getByRole("region", { name: "租赁日历" });
  await calendar.locator("summary").click();
  await expect(
    calendar.getByText("09:00–11:00", { exact: true }),
  ).toBeVisible();
  await calendar
    .getByRole("button", { name: "11:00–24:00", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "保存租赁", exact: true }),
  ).toBeEnabled();
  await expect(page.getByLabel("备注", { exact: true })).toHaveValue(
    "My calendar draft",
  );
  await calendar.locator("summary").click();
  const fields = await page
    .locator(".rental-editor-form input, .rental-editor-form select")
    .evaluateAll((elements) =>
      elements.map((element) => ({
        height: element.getBoundingClientRect().height,
        top: element.getBoundingClientRect().top,
      })),
    );
  expect(fields.every((field) => field.height === 42)).toBe(true);
  expect(fields[0].top).toBe(fields[1].top);
  expect(fields[2].top).toBe(fields[3].top);
  await expect(page.getByRole("link", { name: "返回租赁列表" })).toHaveCount(0);
  await calendar.getByRole("button", { name: "年视图", exact: true }).click();
  await expect(
    calendar.getByRole("button", { name: "12 月", exact: true }),
  ).toBeVisible();
  await calendar.getByRole("button", { name: "日视图", exact: true }).click();
  async function dragCells(
    from: ReturnType<typeof calendar.locator>,
    to: ReturnType<typeof calendar.locator>,
  ) {
    await from.scrollIntoViewIfNeeded();
    await to.scrollIntoViewIfNeeded();
    const a = (await from.boundingBox())!,
      b = (await to.boundingBox())!;
    await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
    await page.mouse.down();
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 10 });
    await page.mouse.up();
  }
  await dragCells(
    calendar.getByRole("button", { name: "12:00–12:30，空闲", exact: true }),
    calendar.getByRole("button", { name: "13:00–13:30，空闲", exact: true }),
  );
  let selected = await page
    .getByLabel("开始时间", { exact: true })
    .inputValue();
  expect(selected).toMatch(/T12:00/);
  await expect(page.getByLabel("结束时间", { exact: true })).toHaveValue(
    /T13:30/,
  );
  await calendar
    .getByRole("button", { name: "14:00–14:30，空闲", exact: true })
    .click();
  await expect(page.getByLabel("开始时间", { exact: true })).toHaveValue(
    /T14:00/,
  );
  await expect(page.getByLabel("结束时间", { exact: true })).toHaveValue(
    /T14:30/,
  );
  selected = await page.getByLabel("开始时间", { exact: true }).inputValue();
  await expect(
    calendar.getByRole("button", {
      name: "09:30–10:00，已有租赁",
      exact: true,
    }),
  ).toBeDisabled();
  await dragCells(
    calendar.getByRole("button", { name: "08:30–09:00，空闲", exact: true }),
    calendar.getByRole("button", {
      name: "09:30–10:00，已有租赁",
      exact: true,
    }),
  );
  await expect(calendar.getByRole("alert")).toContainText("已有租赁");
  await expect(page.getByLabel("开始时间", { exact: true })).toHaveValue(
    selected,
  );
  await calendar.getByRole("button", { name: "月视图", exact: true }).click();
  const range = await page.evaluate(() => {
    const date = new Date();
    const first = new Date(date.getFullYear(), date.getMonth() + 1, 10);
    const last = new Date(first.getFullYear(), first.getMonth(), 12);
    return {
      year: first.getFullYear(),
      month: first.getMonth(),
      first: first.getTime(),
      last: last.getTime(),
    };
  });
  await calendar
    .getByRole("combobox", { name: "显示年份" })
    .selectOption(String(range.year));
  await calendar
    .getByRole("combobox", { name: "显示月份" })
    .selectOption(String(range.month));
  await dragCells(
    calendar.locator(`[data-calendar-start="${range.first}"]`),
    calendar.locator(`[data-calendar-start="${range.last}"]`),
  );
  await expect(page.getByLabel("开始时间", { exact: true })).toHaveValue(
    /-10T00:00/,
  );
  await expect(page.getByLabel("结束时间", { exact: true })).toHaveValue(
    /-13T00:00/,
  );

  const singleDate = await page.evaluate(
    ({ year, month }) => new Date(year, month, 17).getTime(),
    range,
  );
  await calendar.locator('[data-calendar-start="' + singleDate + '"]').click();
  await expect(page.getByLabel("开始时间", { exact: true })).toHaveValue(
    /-17T00:00/,
  );
  await expect(page.getByLabel("结束时间", { exact: true })).toHaveValue(
    /-18T00:00/,
  );
  await expect(calendar.locator(".rental-day.is-selected")).toHaveCount(1);
  const calendarFit = await calendar.evaluate((element) => {
    const body = element.querySelector(".rental-calendar-body")!;
    const grid = body.querySelector(".rental-month-grid")!;
    return {
      body: body.clientHeight,
      grid: grid.getBoundingClientRect().height,
    };
  });
  expect(calendarFit.grid).toBeLessThanOrEqual(calendarFit.body + 1);
  await page.screenshot({
    path: "/tmp/ostool-rental-calendar.png",
    fullPage: true,
  });
  const slot = await page.getByLabel("开始时间", { exact: true }).inputValue();
  const slotEnd = await page
    .getByLabel("结束时间", { exact: true })
    .inputValue();
  expect(
    (
      await request.post(`${base}/api/v1/admin/rentals`, {
        headers,
        data: {
          board_id: boardId,
          user_id: me.id,
          starts_at: new Date(slot).toISOString(),
          ends_at: new Date(slotEnd).toISOString(),
          notes: "Concurrent reservation",
        },
      })
    ).ok(),
  ).toBeTruthy();
  await expect(
    page.getByRole("button", { name: "保存租赁", exact: true }),
  ).toBeDisabled();
  await expect(page.getByLabel("备注", { exact: true })).toHaveValue(
    "My calendar draft",
  );
  for (const route of [
    "boards",
    "dtbs",
    "sessions",
    "users",
    "users/roles",
    "rentals",
    "rental-requests",
  ]) {
    await page.goto(`${base}/admin/${route}`);
    await expect(
      page.getByRole("button", { name: "切换列表布局", exact: true }).first(),
    ).toBeVisible();
  }
  await page.goto(`${base}/admin/boards`);
  await page.getByLabel("搜索开发板", { exact: true }).fill(boardId);
  const head = page.getByRole("columnheader", { name: "开发板", exact: true });
  await expect(head).toBeVisible();
  expect(
    await head.evaluate((element) => getComputedStyle(element).fontSize),
  ).toBe("16px");
  const edit = page.getByRole("link", { name: "编辑", exact: true });
  const size = await edit.evaluate((element) => ({
    width: element.getBoundingClientRect().width,
    height: element.getBoundingClientRect().height,
    icon: getComputedStyle(element.querySelector("svg")!).width,
  }));
  expect(size).toEqual({ width: 42, height: 42, icon: "18px" });
  await page.mouse.move(0, 0);
  const appearance = await edit.evaluate((element) => ({
    background: getComputedStyle(element).backgroundColor,
    border: getComputedStyle(element).borderTopColor,
  }));
  expect(appearance).toEqual({
    background: "rgba(0, 0, 0, 0)",
    border: "rgba(0, 0, 0, 0)",
  });
  await edit.hover();
  await expect
    .poll(() =>
      edit.evaluate((element) => getComputedStyle(element).backgroundColor),
    )
    .not.toBe(appearance.background);

  await page.getByRole("button", { name: "切换列表布局", exact: true }).click();
  await page
    .getByRole("menuitemradio", { name: "卡片视图", exact: true })
    .click();
  await expect(page.getByRole("list", { name: "卡片列表" })).toBeVisible();
  await expect(
    page.getByRole("listitem").filter({ hasText: boardId }),
  ).toHaveCount(1);
  await expect(page.getByLabel("搜索开发板", { exact: true })).toHaveValue(
    boardId,
  );
  await page.screenshot({
    path: "/tmp/ostool-board-cards.png",
    fullPage: true,
  });
  await page.reload();
  await expect(page.getByRole("list", { name: "卡片列表" })).toBeVisible();
});

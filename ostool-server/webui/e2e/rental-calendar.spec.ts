import { expect, test } from "playwright/test";
test("standalone rental calendar, live conflicts and persistent card layouts", async ({
  page,
  request,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires disposable PostgreSQL public server");
  const email = process.env.OSTOOL_E2E_ADMIN_EMAIL ?? "admin@example.com",
    password = process.env.OSTOOL_E2E_ADMIN_PASSWORD ?? "admin-password-123";
  const login = await (
    await request.post(`${base}/api/v1/auth/login`, {
      data: { email, password },
    })
  ).json();
  const headers = { authorization: `Bearer ${login.access_token}` };
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
  await page.goto(`${base}/login?redirect=/admin/rentals`);
  await page.getByLabel("邮箱", { exact: true }).fill(email);
  await page.getByLabel("密码", { exact: true }).fill(password);
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

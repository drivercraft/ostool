import { expect, test } from "playwright/test";
import { apiLogin } from "../fixtures/security";

test("management inventory omits commands while only the selected editor receives live details", async ({
  page,
  context,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires disposable public fixtures");
  await apiLogin(
    context.request,
    base!,
    process.env.OSTOOL_E2E_ADMIN_EMAIL!,
    process.env.OSTOOL_E2E_ADMIN_PASSWORD!,
  );
  const headers = { "x-ostool-surface": "admin", origin: base! };
  const suffix = Date.now();
  const ids = [`privacy-first-${suffix}`, `privacy-second-${suffix}`];
  for (const [index, id] of ids.entries()) {
    const created = await context.request.post(base + "/api/v1/admin/boards", {
      headers,
      data: {
        id,
        board_type: "privacy-review",
        tags: [],
        disabled: false,
        serial: null,
        notes: `PRIVATE_NOTES_${index}`,
        reset_command: `true # PRIVATE_RESTORE_${index}`,
        network_identity: null,
        boot: { kind: "pxe", notes: null },
        power_management: {
          kind: "custom",
          power_on_cmd: `true # PRIVATE_POWER_${index}`,
          power_off_cmd: "true",
        },
      },
    });
    expect(created.ok()).toBe(true);
  }
  try {
    const response = await context.request.get(base + "/api/v1/admin/boards", {
      headers,
    });
    const text = await response.text();
    expect(text).not.toContain("PRIVATE_");
    expect(text).not.toContain("power_management");
    expect(text).not.toContain("reset_command");
    await page.addInitScript(() => {
      const events: string[] = [];
      (window as unknown as { privacyEvents: string[] }).privacyEvents = events;
      const Original = window.EventSource;
      window.EventSource = class extends Original {
        constructor(url: string | URL, options?: EventSourceInit) {
          super(url, options);
          for (const type of ["snapshot", "update"])
            this.addEventListener(type, (event) =>
              events.push((event as MessageEvent).data),
            );
        }
      };
    });
    await page.goto(base + "/admin/boards");
    await expect(
      page.getByRole("link", { name: ids[0], exact: true }),
    ).toBeVisible();
    expect(
      await page.evaluate(() =>
        (window as unknown as { privacyEvents: string[] }).privacyEvents.join(
          "\n",
        ),
      ),
    ).not.toContain("PRIVATE_");
    await page.getByRole("link", { name: ids[0], exact: true }).click();
    const command = page.getByLabel("上电命令", { exact: true });
    await expect(command).toHaveValue("true # PRIVATE_POWER_0");
    expect(
      await page.evaluate(() =>
        (window as unknown as { privacyEvents: string[] }).privacyEvents.join(
          "\n",
        ),
      ),
    ).not.toContain("PRIVATE_POWER_1");
    const notes = page.getByLabel("备注", { exact: true });
    await notes.fill("local-draft");
    const full = await (
      await context.request.get(base + "/api/v1/admin/boards/" + ids[0], {
        headers,
      })
    ).json();
    const changed = await context.request.put(
      base + "/api/v1/admin/boards/" + ids[0],
      { headers, data: { ...full, notes: "peer update" } },
    );
    expect(changed.ok()).toBe(true);
    await expect(
      page.getByRole("button", { name: "载入服务器版本", exact: true }),
    ).toBeVisible();
    await expect(notes).toHaveValue("local-draft");
    await expect(
      page.getByRole("button", { name: "保存开发板", exact: true }),
    ).toBeDisabled();
    await page
      .getByRole("button", { name: "载入服务器版本", exact: true })
      .click();
    await expect(notes).toHaveValue("peer update");
    await expect(command).toHaveValue("true # PRIVATE_POWER_0");
    await notes.fill("saved update");
    await page.getByRole("button", { name: "保存开发板", exact: true }).click();
    await expect(page.getByText("已保存开发板", { exact: true })).toBeVisible();
    await page.reload();
    await expect(command).toHaveValue("true # PRIVATE_POWER_0");
    await expect(notes).toHaveValue("saved update");
    await page.getByRole("link", { name: "取消", exact: true }).click();
    await page.getByRole("link", { name: ids[1], exact: true }).click();
    await expect(command).toHaveValue("true # PRIVATE_POWER_1");
  } finally {
    for (const id of ids)
      expect(
        (
          await context.request.delete(base + "/api/v1/admin/boards/" + id, {
            headers,
          })
        ).ok(),
      ).toBe(true);
  }
});

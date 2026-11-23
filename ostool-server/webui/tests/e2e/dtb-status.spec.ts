import { expect, test } from "../fixtures/local";

test("DTB disable updates the list and prevents board selection until re-enabled", async ({
  page,
  request,
}) => {
  const base = "http://127.0.0.1:4175/api/v1/admin/dtbs";
  const name = "status-browser.dtb";
  expect(
    (
      await request.post(base, {
        headers: {
          "X-Dtb-Name": name,
          "Content-Type": "application/octet-stream",
        },
        data: Buffer.from("dtb fixture"),
      })
    ).status(),
  ).toBe(201);
  try {
    await page.goto("/qcl/dtbs");
    await page.getByLabel("搜索 DTB").fill(name);
    const row = page.getByRole("row").filter({ hasText: name });
    await expect(row.getByRole("button")).toHaveCount(3);
    await row.getByRole("button", { name: "禁用", exact: true }).click();
    await expect(row.getByText("已禁用", { exact: true })).toBeVisible();
    await expect(
      row.getByRole("button", { name: "启用", exact: true }),
    ).toBeVisible();
    await page.goto("/qcl/boards/new");
    const option = page.getByRole("option", {
      name: `${name}（已禁用）`,
      exact: true,
    });
    await expect(option).toBeDisabled();
    expect(
      (
        await request.patch(`${base}/${name}`, { data: { disabled: false } })
      ).status(),
    ).toBe(200);
    await expect(page.getByRole("option", { name, exact: true })).toBeEnabled();
  } finally {
    await request.delete(`${base}/${name}`);
  }
});

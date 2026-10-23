import { expect, test } from "playwright/test";
import { solveCaptcha } from "../fixtures/security";
test("system settings write encrypted credentials without echoing them and protect secret drafts from concurrent updates", async ({
  page,
  context,
}) => {
  const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
  test.skip(!base, "requires disposable public fixtures");
  await page.goto(base + "/admin/login?redirect=/admin/settings");
  await page
    .getByLabel("账号", { exact: true })
    .fill(process.env.OSTOOL_E2E_ADMIN_EMAIL!);
  await page
    .getByLabel("密码", { exact: true })
    .fill(process.env.OSTOOL_E2E_ADMIN_PASSWORD!);
  await solveCaptcha(page);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(page).toHaveURL(base + "/admin/settings");
  const headers = { "x-ostool-surface": "admin", origin: base! };
  const endpoint = base + "/api/v1/admin/account-settings";
  const initial = await (
    await context.request.get(endpoint, { headers })
  ).json();
  await page.getByRole("tab", { name: "邮件服务", exact: true }).click();
  const password = page.getByLabel("SMTP 密码", { exact: true });
  await expect(password).toHaveValue("");
  await page.getByLabel("SMTP 主机", { exact: true }).fill("smtp.example.test");
  await page.getByLabel("发件人", { exact: true }).fill("noreply@example.test");
  await page.getByLabel("SMTP 用户名", { exact: true }).fill("smtp-test");
  await password.fill("browser-private-smtp-password");
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(password).toHaveValue("");
  await expect(password).toHaveAttribute("placeholder", /已配置/);
  await page.reload();
  await expect(password).toHaveValue("");
  const current = await (
    await context.request.get(endpoint, { headers })
  ).json();
  expect(current.secrets.smtp_password).toBe(true);
  expect(JSON.stringify(current)).not.toContain(
    "browser-private-smtp-password",
  );
  await password.fill("pending-secret-draft");
  const updated = await context.request.put(endpoint, {
    headers,
    data: {
      revision: current.revision,
      settings: { ...current.settings, login_max_failures: 7 },
    },
  });
  expect(updated.ok()).toBe(true);
  await expect(page.getByText(/当前策略已被修改/)).toBeVisible();
  await expect(password).toHaveValue("pending-secret-draft");
  await expect(
    page.getByRole("button", { name: "保存配置", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "取消", exact: true }).click();
  const latest = await (
    await context.request.get(endpoint, { headers })
  ).json();
  expect(
    (
      await context.request.put(endpoint, {
        headers,
        data: {
          revision: latest.revision,
          settings: initial.settings,
          secret_updates: { clear_smtp_password: true },
        },
      })
    ).ok(),
  ).toBe(true);
  await expect(password).toHaveAttribute("placeholder", /未配置/);
});

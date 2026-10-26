import { expect, test } from "playwright/test";
import { apiLogin, solveCaptcha } from "../fixtures/security";
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

test("reserved name lists preserve multiline drafts, save through SSE and persist after reload", async ({
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
  const endpoint = base + "/api/v1/admin/account-settings";
  const initial = await (
    await context.request.get(endpoint, { headers })
  ).json();
  try {
    await page.goto(base + "/admin/settings?tab=security");
    const username = page.getByLabel("用户名禁用关键词", { exact: true });
    const nickname = page.getByLabel("昵称禁用关键词", { exact: true });
    await expect(username).toHaveValue(
      initial.settings.name_policy.username_keywords.join("\n"),
    );
    await expect(nickname).toHaveValue(
      initial.settings.name_policy.display_name_keywords.join("\n"),
    );
    const words = "admin\nadministrator\nplatform_owner\n";
    await username.fill(words);
    await nickname.fill("管理员\n官方\n平台代表\n");
    // A resource update unrelated to the edited lists must not discard trailing newlines/drafts.
    const update = await context.request.put(endpoint, {
      headers,
      data: {
        revision: initial.revision,
        settings: {
          ...initial.settings,
          login_lock_secs: initial.settings.login_lock_secs + 1,
        },
      },
    });
    expect(update.ok()).toBe(true);
    await expect(
      page
        .getByRole("tabpanel", { name: "登录与安全" })
        .getByText(new RegExp("策略版本 " + (initial.revision + 1))),
    ).toBeVisible();
    await expect(username).toHaveValue(words);
    await page.getByRole("button", { name: "保存配置", exact: true }).click();
    await expect(page.getByText("策略已保存", { exact: true })).toBeVisible();
    await expect(username).toHaveValue("admin\nadministrator\nplatform_owner");
    await page.reload();
    await expect(username).toHaveValue("admin\nadministrator\nplatform_owner");
    await expect(nickname).toHaveValue("管理员\n官方\n平台代表");
    const current = await (
      await context.request.get(endpoint, { headers })
    ).json();
    expect(current.settings.name_policy.username_keywords).toEqual([
      "admin",
      "administrator",
      "platform_owner",
    ]);
    expect(current.settings.name_policy.display_name_keywords).toEqual([
      "管理员",
      "官方",
      "平台代表",
    ]);
    await username.fill("private_draft\n");
    await page.getByRole("button", { name: "取消", exact: true }).click();
    await expect(username).toHaveValue("admin\nadministrator\nplatform_owner");
  } finally {
    const current = await (
      await context.request.get(endpoint, { headers })
    ).json();
    expect(
      (
        await context.request.put(endpoint, {
          headers,
          data: { revision: current.revision, settings: initial.settings },
        })
      ).ok(),
    ).toBe(true);
  }
});

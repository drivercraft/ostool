import { expect, test, type Locator } from "playwright/test";

async function appearance(control: Locator) {
  return control.evaluate((element) => {
    const style = getComputedStyle(element);
    return Object.fromEntries(
      [
        "height",
        "borderRadius",
        "borderTopWidth",
        "borderTopColor",
        "backgroundColor",
        "color",
        "fontFamily",
        "fontSize",
        "fontWeight",
        "lineHeight",
        "boxShadow",
      ].map((key) => [key, style[key as keyof CSSStyleDeclaration]]),
    );
  });
}
for (const theme of ["light", "dark"] as const) {
  test(`${theme}: website, authentication and admin controls share normal, focus, invalid and disabled styles`, async ({
    browser,
  }) => {
    const base = process.env.OSTOOL_E2E_AUTH_BASE_URL;
    test.skip(!base, "requires disposable public fixtures");
    const context = await browser.newContext({ colorScheme: theme });
    await context.addInitScript((value) => {
      localStorage.setItem("ostool.web.theme", value);
      localStorage.setItem("ostool.admin.theme", value);
    }, theme);
    const snapshots: Record<string, unknown>[] = [];
    const buttons: Record<string, unknown>[] = [];
    try {
      for (const [path, label] of [
        [base + "/login", "账号"],
        [base + "/register", "用户名"],
        [base + "/admin/login", "账号"],
        [base + "/devices", "搜索设备"],
        ["http://127.0.0.1:4174/admin/boards/new", "板型"],
      ]) {
        const page = await context.newPage();
        await page.goto(path);
        const input = page.getByLabel(label, { exact: true });
        await expect(input).toBeEnabled();
        await expect
          .poll(() =>
            page
              .locator("html")
              .evaluate((el) => el.classList.contains("dark")),
          )
          .toBe(theme === "dark");
        const normal = await appearance(input);
        await input.focus();
        await expect(input).toBeFocused();
        // Wait for CSS transitions before comparing computed colors.
        await expect
          .poll(() => input.evaluate((el) => getComputedStyle(el).boxShadow))
          .not.toBe("none");
        await page.waitForTimeout(250);
        const focused = await appearance(input);
        await input.evaluate((el) => el.setAttribute("aria-invalid", "true"));
        await page.waitForTimeout(250);
        const invalid = await appearance(input);
        expect(invalid.borderTopColor).not.toBe(focused.borderTopColor);
        await input.evaluate((el) => {
          el.removeAttribute("aria-invalid");
          (el as HTMLInputElement).disabled = true;
        });
        await page.waitForTimeout(250);
        const disabled = await appearance(input);
        snapshots.push({ normal, focused, invalid, disabled });
        if (
          path.endsWith("/login") ||
          path.endsWith("/register") ||
          path.endsWith("/boards/new")
        ) {
          const submit = page
            .locator('button[type="submit"][data-slot="button"]')
            .first();
          if (await submit.count()) {
            await submit.evaluate(
              (el) => ((el as HTMLButtonElement).disabled = false),
            );
            buttons.push(await appearance(submit));
          }
        }
        if (path.endsWith("/boards/new")) {
          for (const slot of ["native-select", "textarea"]) {
            const control = page.locator(`[data-slot="${slot}"]`).first();
            await control.focus();
            await page.waitForTimeout(250);
            const style = await appearance(control);
            expect(style.borderTopColor).toBe(focused.borderTopColor);
            expect(style.boxShadow).toBe(focused.boxShadow);
            expect(style.borderRadius).toBe(focused.borderRadius);
          }
        }
        await page.close();
      }
      for (const snapshot of snapshots.slice(1))
        expect(snapshot).toEqual(snapshots[0]);
      for (const button of buttons.slice(1)) expect(button).toEqual(buttons[0]);
    } finally {
      await context.close();
    }
  });
}

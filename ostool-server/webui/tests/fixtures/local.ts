import { test as base } from "playwright/test";
export { expect } from "playwright/test";
// Only fixture API calls select the administrator cookie. Browser requests keep
// the application's real website/admin surface headers and CSRF behavior.
export const test = base.extend({
  request: async ({ playwright }, use) => {
    const request = await playwright.request.newContext({
      storageState: process.env.OSTOOL_E2E_LOCAL_AUTH_STATE,
      extraHTTPHeaders: {
        origin: "http://127.0.0.1:4174",
        "x-ostool-surface": "admin",
      },
    });
    try {
      await use(request);
    } finally {
      await request.dispose();
    }
  },
});

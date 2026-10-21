import { afterEach, describe, expect, it, vi } from "vitest";
import { api, clearBrowserSession, notifyAuthChanged } from "./api/client";
import { sessionSignalKey, safeAdminRedirect } from "./auth-session";

afterEach(() => {
  localStorage.clear();
  vi.unstubAllGlobals();
  window.history.replaceState({}, "", "/");
});
describe("independent browser sessions", () => {
  it("admin login and logout leave the website session valid", async () => {
    window.history.replaceState({}, "", "/login");
    notifyAuthChanged();
    const website = localStorage.getItem(sessionSignalKey("website"));
    window.history.replaceState({}, "", "/admin/login");
    notifyAuthChanged();
    const fetch = vi
      .fn()
      .mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetch);
    await api.logout();
    expect(fetch.mock.calls[0][0]).toBe("/api/v1/auth/logout");
    expect(fetch.mock.calls[0][1].headers["x-ostool-surface"]).toBe("admin");
    expect(fetch.mock.calls[0][1].body).toBeUndefined();
    expect(localStorage.getItem(sessionSignalKey("admin"))).toBeNull();
    expect(localStorage.getItem(sessionSignalKey("website"))).toBe(website);
    notifyAuthChanged();
    const admin = localStorage.getItem(sessionSignalKey("admin"));
    window.history.replaceState({}, "", "/account");
    clearBrowserSession();
    expect(localStorage.getItem(sessionSignalKey("admin"))).toBe(admin);
    expect(localStorage.getItem(sessionSignalKey("website"))).toBeNull();
    expect(
      Object.keys(localStorage).some((key) =>
        /access_token|refresh_token/.test(key),
      ),
    ).toBe(false);
  });
  it("backend redirects stay within backend routes", () => {
    for (const value of [
      null,
      "https://example.com",
      "//example.com",
      "/account",
      "/admin/../account",
      "/admin/login?redirect=/admin/login",
      "/admin/\\example.com",
    ]) {
      expect(safeAdminRedirect(value)).toBe("/admin/overview");
    }
    expect(safeAdminRedirect("/admin/boards/demo?view=details")).toBe(
      "/admin/boards/demo?view=details",
    );
  });
});

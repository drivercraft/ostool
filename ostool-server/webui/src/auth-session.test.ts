import { afterEach, describe, expect, it, vi } from "vitest";
import {
  api,
  clearAuthToken,
  saveAuthToken,
  type AuthToken,
} from "./api/client";
import { accessToken, safeAdminRedirect } from "./auth-session";

afterEach(() => {
  localStorage.clear();
  vi.unstubAllGlobals();
  window.history.replaceState({}, "", "/");
});
describe("independent browser sessions", () => {
  it("admin login and logout leave the website session valid", async () => {
    const grant = (id: string) =>
      ({ access_token: id, refresh_token: `${id}-refresh` }) as AuthToken;
    window.history.replaceState({}, "", "/login");
    saveAuthToken(grant("website"));
    window.history.replaceState({}, "", "/admin/login");
    expect(accessToken()).toBeNull();
    saveAuthToken(grant("admin"));
    const fetch = vi
      .fn()
      .mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetch);
    await api.logout();
    expect(new URLSearchParams(fetch.mock.calls[0][1].body).get("token")).toBe(
      "admin-refresh",
    );
    expect(accessToken("admin")).toBeNull();
    expect(accessToken("website")).toBe("website");
    saveAuthToken(grant("admin-again"));
    window.history.replaceState({}, "", "/account");
    clearAuthToken();
    expect(accessToken("admin")).toBe("admin-again");
    expect(accessToken("website")).toBeNull();
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

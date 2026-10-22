import type { AuthConfig } from "@/api/client";
export function readAuthBootstrap(): AuthConfig | undefined {
  const element = document.getElementById("ostool-bootstrap");
  if (!element?.textContent) return;
  try {
    const value = JSON.parse(element.textContent) as AuthConfig;
    if (
      ["local", "public"].includes(value.edition) &&
      typeof value.enabled === "boolean"
    )
      return value;
  } catch {
    /* The regular API remains available if HTML bootstrap data is absent. */
  }
}

export type AuthSurface = "admin" | "website";
let applicationSurface: AuthSurface | undefined;
export function configureAuthSurface(surface: AuthSurface) {
  applicationSurface = surface;
  // Browser credentials are exclusively HttpOnly cookies.
  for (const key of [
    "ostool.access_token",
    "ostool.refresh_token",
    "ostool.admin.access_token",
    "ostool.admin.refresh_token",
  ])
    localStorage.removeItem(key);
}
export const authSurface = (): AuthSurface =>
  applicationSurface ??
  (window.location.pathname === "/admin" ||
  window.location.pathname.startsWith("/admin/")
    ? "admin"
    : "website");
export const authEvent = (surface: AuthSurface = authSurface()) =>
  surface === "admin" ? "ostool:admin-auth" : "ostool:auth";
export const sessionSignalKey = (surface: AuthSurface = authSurface()) =>
  `ostool.${surface}.session-state`;
export function notifyAuthChanged() {
  localStorage.setItem(sessionSignalKey(), crypto.randomUUID());
  window.dispatchEvent(new Event(authEvent()));
}
export function clearBrowserSession() {
  const present = localStorage.getItem(sessionSignalKey()) !== null;
  localStorage.removeItem(sessionSignalKey());
  if (present) window.dispatchEvent(new Event(authEvent()));
}
export function loginUrl(surface: AuthSurface = authSurface()) {
  if (surface === "website") return "/login";
  const redirect = window.location.pathname + window.location.search;
  return `/admin/login?redirect=${encodeURIComponent(safeAdminRedirect(redirect))}`;
}
export function safeAdminRedirect(value: string | null) {
  if (!value || !value.startsWith("/admin/") || /[\\\u0000-\u0020]/.test(value))
    return "/admin/overview";
  const path = new URL(value, window.location.origin);
  return path.origin === window.location.origin &&
    path.pathname.startsWith("/admin/") &&
    path.pathname !== "/admin/login" &&
    path.pathname !== "/admin/login/"
    ? path.pathname + path.search + path.hash
    : "/admin/overview";
}

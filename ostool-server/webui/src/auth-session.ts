export type AuthSurface = "admin" | "website";
let applicationSurface: AuthSurface | undefined;
export function configureAuthSurface(surface: AuthSurface) {
  applicationSurface = surface;
}
export const authSurface = (): AuthSurface =>
  applicationSurface ??
  (window.location.pathname === "/admin" ||
  window.location.pathname.startsWith("/admin/")
    ? "admin"
    : "website");
export const authKeys = (surface: AuthSurface = authSurface()) => ({
  access:
    surface === "admin" ? "ostool.admin.access_token" : "ostool.access_token",
  refresh:
    surface === "admin" ? "ostool.admin.refresh_token" : "ostool.refresh_token",
  event: surface === "admin" ? "ostool:admin-auth" : "ostool:auth",
});
export const accessToken = (surface?: AuthSurface) =>
  localStorage.getItem(authKeys(surface).access);
export const refreshToken = (surface?: AuthSurface) =>
  localStorage.getItem(authKeys(surface).refresh);
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

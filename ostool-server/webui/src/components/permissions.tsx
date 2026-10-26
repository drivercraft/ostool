import { createContext, useContext, type ReactNode } from "react";
import type { AuthUser } from "@/api/client";
const Context = createContext<{
  can: (code: string) => boolean;
  user?: AuthUser;
  local: boolean;
}>({
  can: () => true,
  local: true,
});
export function PermissionsProvider({
  user,
  local,
  children,
}: {
  user?: AuthUser;
  local: boolean;
  children: ReactNode;
}) {
  const can = (code: string) =>
    local ||
    !!user?.is_admin ||
    !!user?.permissions?.includes("system.admin") ||
    !!user?.permissions?.includes(code);
  return (
    <Context.Provider value={{ can, user, local }}>{children}</Context.Provider>
  );
}
export const usePermissions = () => useContext(Context);
export function pagePermission(path: string, manage = false): string {
  if (path === "/" || path === "/dashboard") return "console.access";
  if (path.startsWith("/server") || path.startsWith("/tftp"))
    return manage ? "server.manage" : "server.read";
  if (path.startsWith("/boards/")) return "devices.manage";
  if (path.startsWith("/boards"))
    return manage ? "devices.manage" : "devices.read";
  if (path.startsWith("/dtbs")) return manage ? "dtbs.manage" : "dtbs.read";
  if (path.startsWith("/sessions"))
    return manage ? "sessions.manage" : "sessions.read";
  if (path.startsWith("/users/roles"))
    return manage ? "roles.manage" : "roles.read";
  if (path.startsWith("/users")) return manage ? "users.manage" : "users.read";
  if (path.startsWith("/rental-requests"))
    return manage ? "requests.manage" : "requests.read";
  if (path.startsWith("/rentals/")) return "rentals.manage";
  if (path.startsWith("/rentals"))
    return manage ? "rentals.manage" : "rentals.read";
  if (path.startsWith("/audits"))
    return manage ? "audits.manage" : "audits.read";
  if (path.startsWith("/settings")) return "console.access";
  return "dashboard.read";
}
export function firstPage(can: (code: string) => boolean): string {
  return (
    [
      "/overview",
      "/boards",
      "/dtbs",
      "/sessions",
      "/rentals",
      "/rental-requests",
      "/users",
      "/users/roles",
      "/audits",
    ].find((path) => can(pagePermission(path))) ?? "/settings"
  );
}

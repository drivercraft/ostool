import { describe, expect, it } from "vitest";
import type { AuthUser } from "@/api/client";
import { firstPage, hasPermission, pagePermission } from "./permissions";

const administrator: AuthUser = {
  id: "admin",
  username: "admin",
  email: "admin@example.test",
  display_name: null,
  is_admin: true,
};

describe("edition capabilities", () => {
  it("keeps anonymous device, session, audit and server management in local edition", () => {
    for (const code of [
      "console.access",
      "dashboard.read",
      "devices.manage",
      "devices.power",
      "dtbs.manage",
      "ota.manage",
      "sessions.manage",
      "audits.manage",
      "site.manage",
      "server.manage",
    ])
      expect(hasPermission(undefined, true, code), code).toBe(true);
  });
  it("disables identity, registration security and rental capabilities even for a stored administrator", () => {
    for (const code of [
      "users.read",
      "users.manage",
      "roles.manage",
      "rentals.read",
      "rentals.manage",
      "requests.manage",
      "security.read",
      "security.manage",
      "system.admin",
    ])
      expect(hasPermission(administrator, true, code), code).toBe(false);
    for (const path of [
      "/users",
      "/users/roles/new",
      "/users/roles/member",
      "/rentals/new",
      "/rental-requests",
    ])
      expect(hasPermission(undefined, true, pagePermission(path)), path).toBe(
        false,
      );
    expect(firstPage((code) => hasPermission(undefined, true, code))).toBe(
      "/overview",
    );
  });
  it("preserves public edition role permissions and administrator access", () => {
    expect(hasPermission(undefined, false, "devices.read")).toBe(false);
    const member = {
      ...administrator,
      is_admin: false,
      permissions: ["devices.read"],
    };
    expect(hasPermission(member, false, "devices.read")).toBe(true);
    expect(hasPermission(member, false, "users.manage")).toBe(false);
    expect(hasPermission(administrator, false, "security.manage")).toBe(true);
    expect(
      hasPermission(
        { ...member, permissions: ["system.admin"] },
        false,
        "roles.manage",
      ),
    ).toBe(true);
  });
});

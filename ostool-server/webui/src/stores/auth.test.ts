import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  getCurrentUser: vi.fn(),
  login: vi.fn(),
  logout: vi.fn(),
}));

vi.mock("@/api", () => ({
  api: {
    getCurrentUser: mocks.getCurrentUser,
    login: mocks.login,
    logout: mocks.logout,
  },
}));

import { useAuthStore } from "./auth";

const demoUser = {
  id: "user-demo",
  username: "demo",
  display_name: "Demo",
  nickname: null,
  avatar_url: null,
  email: "demo@ostool.local",
  phone: null,
  department: null,
  title: null,
  last_login_at: null,
  roles: [{ id: "role-user", name: "user", display_name: "普通用户", description: "", system: true, permissions: [], created_at: "", updated_at: "" }],
  permissions: [],
};

const adminUser = {
  ...demoUser,
  id: "user-admin",
  username: "admin",
  roles: [{ id: "role-admin", name: "admin", display_name: "管理员", description: "", system: true, permissions: [], created_at: "", updated_at: "" }],
  permissions: [{ id: "perm-settings", code: "settings.manage", name: "管理系统设置", description: "" }],
};

describe("useAuthStore", () => {
  beforeEach(() => {
    mocks.getCurrentUser.mockReset();
    mocks.login.mockReset();
    mocks.logout.mockReset();
  });

  it("loads the current user from the backend", async () => {
    mocks.getCurrentUser.mockResolvedValue(demoUser);
    const store = useAuthStore();

    await store.loadCurrentUser();

    expect(store.isAuthenticated).toBe(true);
    expect(store.user?.username).toBe("demo");
  });

  it("treats failed current-user loading as logged out", async () => {
    mocks.getCurrentUser.mockRejectedValue(new Error("401"));
    const store = useAuthStore();

    await store.loadCurrentUser();

    expect(store.loaded).toBe(true);
    expect(store.isAuthenticated).toBe(false);
  });

  it("logs in and exposes admin role", async () => {
    mocks.login.mockResolvedValue(adminUser);
    const store = useAuthStore();

    await store.login("admin", "secret");

    expect(mocks.login).toHaveBeenCalledWith({ username: "admin", password: "secret" });
    expect(store.isAuthenticated).toBe(true);
    expect(store.isAdmin).toBe(true);
  });

  it("logs out through the backend and clears user state", async () => {
    mocks.login.mockResolvedValue(demoUser);
    mocks.logout.mockResolvedValue(undefined);
    const store = useAuthStore();
    await store.login("demo", "secret");

    await store.logoutUser();

    expect(mocks.logout).toHaveBeenCalled();
    expect(store.isAuthenticated).toBe(false);
  });
});

import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  getCurrentUser: vi.fn(),
  login: vi.fn(),
  logout: vi.fn(),
}));

vi.mock("@/api", () => ({
  api: {
    auth: {
      getCurrentUser: mocks.getCurrentUser,
      login: mocks.login,
      logout: mocks.logout,
    },
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
  roles: [{ id: "role-user", name: "user", display_name: "注册用户", description: "", system: true, disabled: false, user_count: 0, permissions: [], created_at: "", updated_at: "" }],
  permissions: [],
};

const adminUser = {
  ...demoUser,
  id: "user-admin",
  username: "admin",
  roles: [{ id: "role-admin", name: "admin", display_name: "管理员", description: "", system: true, disabled: false, user_count: 0, permissions: [], created_at: "", updated_at: "" }],
  permissions: [{ id: "perm-server", code: "server.update", name: "编辑服务器配置", description: "" }],
};

const rentalOperatorUser = {
  ...demoUser,
  id: "user-rentals",
  username: "rental-operator",
  permissions: [{ id: "perm-leases-delete", code: "leases.delete", name: "删除租赁", description: "" }],
};

const defaultRentalUser = {
  ...demoUser,
  id: "user-default-rentals",
  username: "default-rental-user",
  permissions: [
    { id: "perm-leases-read", code: "leases.read", name: "查看租赁", description: "" },
    { id: "perm-leases-create", code: "leases.create", name: "新增租赁", description: "" },
    { id: "perm-sessions-read", code: "sessions.read", name: "查看租约会话", description: "" },
    { id: "perm-sessions-create", code: "sessions.create", name: "新增租约会话", description: "" },
  ],
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

    await store.login("admin", "secret", "captcha-token", "captcha-answer");

    expect(mocks.login).toHaveBeenCalledWith({
      username: "admin",
      password: "secret",
      captcha_token: "captcha-token",
      captcha_answer: "captcha-answer",
    });
    expect(store.isAuthenticated).toBe(true);
    expect(store.isAdmin).toBe(true);
  });

  it("allows admin area access for users with scoped admin permissions", async () => {
    mocks.login.mockResolvedValue(rentalOperatorUser);
    const store = useAuthStore();

    await store.login("rental-operator", "secret", "captcha-token", "captcha-answer");

    expect(store.isAdmin).toBe(true);
    expect(store.hasPermission("leases.delete")).toBe(true);
    expect(store.hasPermission("sessions.delete")).toBe(false);
    expect(store.hasPermission("users.delete")).toBe(false);
  });

  it("does not expose admin area for default rental permissions", async () => {
    mocks.login.mockResolvedValue(defaultRentalUser);
    const store = useAuthStore();

    await store.login("default-rental-user", "secret", "captcha-token", "captcha-answer");

    expect(store.isAuthenticated).toBe(true);
    expect(store.isAdmin).toBe(false);
    expect(store.hasPermission("leases.read")).toBe(true);
    expect(store.hasPermission("leases.create")).toBe(true);
    expect(store.hasPermission("sessions.read")).toBe(true);
    expect(store.hasPermission("sessions.create")).toBe(true);
  });

  it("logs out through the backend and clears user state", async () => {
    mocks.login.mockResolvedValue(demoUser);
    mocks.logout.mockResolvedValue(undefined);
    const store = useAuthStore();
    await store.login("demo", "secret", "captcha-token", "captcha-answer");

    await store.logoutUser();

    expect(mocks.logout).toHaveBeenCalled();
    expect(store.isAuthenticated).toBe(false);
  });
});

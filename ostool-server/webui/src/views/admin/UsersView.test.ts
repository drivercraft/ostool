import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AdminRoleResponse, AdminUserResponse } from "@/types/api";

const listAdminUsers = vi.fn();
const listAdminRoles = vi.fn();
const getAdminUserRoles = vi.fn();
const createAdminUser = vi.fn();
const updateAdminUser = vi.fn();
const disableAdminUser = vi.fn();
const resetAdminUserPassword = vi.fn();
const updateAdminUserRoles = vi.fn();

const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
  confirm: vi.fn(),
};

vi.mock("@/api", () => ({
  api: {
    listAdminUsers,
    listAdminRoles,
    getAdminUserRoles,
    createAdminUser,
    updateAdminUser,
    disableAdminUser,
    resetAdminUserPassword,
    updateAdminUserRoles,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

function makeUser(overrides: Partial<AdminUserResponse> = {}): AdminUserResponse {
  return {
    id: "u-1",
    username: "alice",
    display_name: "Alice",
    nickname: null,
    avatar_url: null,
    email: "alice@example.com",
    phone: null,
    department: null,
    title: null,
    disabled: false,
    last_login_at: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function makeRole(id: string, name: string, display: string): AdminRoleResponse {
  return {
    id,
    name,
    display_name: display,
    description: "",
    system: false,
    permissions: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  };
}

describe("UsersView", () => {
  beforeEach(() => {
    [
      listAdminUsers,
      listAdminRoles,
      getAdminUserRoles,
      createAdminUser,
      updateAdminUser,
      disableAdminUser,
      resetAdminUserPassword,
      updateAdminUserRoles,
    ].forEach((fn) => fn.mockReset());
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);

    listAdminUsers.mockResolvedValue({
      users: [
        makeUser({ id: "u-1", username: "alice", display_name: "Alice" }),
        makeUser({
          id: "u-2",
          username: "bob",
          display_name: "Bob",
          disabled: true,
        }),
      ],
    });
    listAdminRoles.mockResolvedValue({
      roles: [makeRole("r-1", "admin", "管理员")],
    });
    getAdminUserRoles.mockResolvedValue({ roles: [] });
  });

  it("renders the toolbar with 新增用户 on the left and search/filters on the right", async () => {
    const UsersView = (await import("./UsersView.vue")).default;
    const wrapper = mount(UsersView);

    await flushPromises();

    const toolbar = wrapper.find(".admin-toolbar");
    expect(toolbar.exists()).toBe(true);
    expect(wrapper.find(".admin-toolbar-left").text()).toContain("新增用户");
    expect(wrapper.find(".admin-toolbar-right .search-field").exists()).toBe(true);
    expect(wrapper.findAll(".admin-toolbar-right .filter-field").length).toBe(2);
  });

  it("renders a user per row with edit / toggle / more actions", async () => {
    const UsersView = (await import("./UsersView.vue")).default;
    const wrapper = mount(UsersView);

    await flushPromises();

    const rows = wrapper.findAll("tbody tr");
    expect(rows.length).toBe(2);
    expect(rows[0].find(".user-cell-username").text()).toBe("alice");
    // 三组核心按钮：edit、toggle、more
    expect(rows[0].findAll(".btn-icon-only").length).toBe(3);
    expect(rows[0].find('button[title="编辑"]').exists()).toBe(true);
    expect(rows[0].find('button[title="禁用"]').exists()).toBe(true);
    expect(rows[0].find('button[title="更多"]').exists()).toBe(true);
  });

  it("filters users by status (disabled)", async () => {
    const UsersView = (await import("./UsersView.vue")).default;
    const wrapper = mount(UsersView);

    await flushPromises();

    expect(wrapper.findAll("tbody tr").length).toBe(2);

    const statusSelect = wrapper
      .findAll(".admin-toolbar-right select")
      .find((s) => s.findAll("option").some((o) => o.text() === "已禁用"));
    await statusSelect!.setValue("disabled");
    await flushPromises();

    const rows = wrapper.findAll("tbody tr");
    expect(rows.length).toBe(1);
    expect(rows[0].find(".user-cell-username").text()).toBe("bob");
  });

  it("opens the create-user modal when 新增用户 is clicked", async () => {
    const UsersView = (await import("./UsersView.vue")).default;
    const wrapper = mount(UsersView);

    await flushPromises();

    expect(wrapper.find(".modal-overlay").exists()).toBe(false);

    await wrapper.find(".admin-toolbar-left .btn.btn-primary").trigger("click");

    const modal = wrapper.find(".modal-overlay");
    expect(modal.exists()).toBe(true);
    expect(modal.text()).toContain("新增用户");
    expect(modal.find('input[autocomplete="off"]').exists()).toBe(true);
  });

  it("calls disableAdminUser when toggling an active user", async () => {
    const UsersView = (await import("./UsersView.vue")).default;
    const wrapper = mount(UsersView);

    await flushPromises();

    const firstRow = wrapper.findAll("tbody tr")[0];
    await firstRow.find('button[title="禁用"]').trigger("click");
    await flushPromises();

    expect(disableAdminUser).toHaveBeenCalledWith("u-1");
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已禁用用户 alice");
  });

  it("reveals the more menu with 编辑用户 / 重置密码 entries", async () => {
    const UsersView = (await import("./UsersView.vue")).default;
    const wrapper = mount(UsersView);

    await flushPromises();

    const firstRow = wrapper.findAll("tbody tr")[0];
    expect(firstRow.find(".action-menu").exists()).toBe(false);

    await firstRow.find('button[title="更多"]').trigger("click");
    await flushPromises();

    const menu = wrapper.find(".action-menu");
    expect(menu.exists()).toBe(true);
    const labels = menu.findAll(".action-menu-item").map((b) => b.text());
    expect(labels.some((t) => t.includes("编辑用户"))).toBe(true);
    expect(labels.some((t) => t.includes("重置密码"))).toBe(true);
  });
});

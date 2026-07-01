import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AdminRoleResponse, AdminUserResponse } from "@/types/api";

const listAdminUsers = vi.fn();
const listAdminRoles = vi.fn();
const getAdminUserRoles = vi.fn();
const createAdminUser = vi.fn();
const updateAdminUser = vi.fn();
const deleteAdminUser = vi.fn();
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
    admin: {
      listAdminUsers,
      listAdminRoles,
      getAdminUserRoles,
      createAdminUser,
      updateAdminUser,
      deleteAdminUser,
      disableAdminUser,
      resetAdminUserPassword,
      updateAdminUserRoles,
    },
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
    status: "active",
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
    disabled: false,
    user_count: 0,
    permissions: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  };
}

function makeDisabledRole(id: string, name: string, display: string): AdminRoleResponse {
  return {
    ...makeRole(id, name, display),
    disabled: true,
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
      deleteAdminUser,
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
      roles: [
        makeRole("r-1", "lab-admin", "实验室管理员"),
        makeRole("r-2", "developer", "开发人员"),
      ],
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
    expect(rows[0].find(".table-cell-main").text()).toBe("alice");
    // 三组核心按钮：edit、toggle、more
    expect(rows[0].findAll(".btn-icon-only").length).toBe(3);
    expect(rows[0].find('button[title="编辑"]').exists()).toBe(true);
    expect(rows[0].find('button[title="禁用"]').exists()).toBe(true);
    expect(rows[0].find('button[title="更多"]').exists()).toBe(true);
  });

  it("renders the row action menu outside the table scroll container", async () => {
    const UsersView = (await import("./UsersView.vue")).default;
    const wrapper = mount(UsersView, {
      attachTo: document.body,
    });

    await flushPromises();

    await wrapper.find('button[title="更多"]').trigger("click");
    await flushPromises();

    expect(wrapper.find(".table-scroll .action-menu").exists()).toBe(false);
    expect(document.body.querySelector(".action-menu.action-menu--floating")).not.toBeNull();

    wrapper.unmount();
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
    expect(rows[0].find(".table-cell-main").text()).toBe("bob");
  });

  it("treats users with disabled roles as unavailable", async () => {
    listAdminRoles.mockResolvedValue({
      roles: [
        makeRole("r-1", "lab-admin", "实验室管理员"),
        makeDisabledRole("r-2", "developer", "开发人员"),
      ],
    });
    getAdminUserRoles.mockImplementation((userId: string) => Promise.resolve({
      roles: userId === "u-1" ? [makeDisabledRole("r-2", "developer", "开发人员")] : [],
    }));

    const UsersView = (await import("./UsersView.vue")).default;
    const wrapper = mount(UsersView);

    await flushPromises();

    const aliceRow = wrapper.findAll("tbody tr")[0];
    expect(aliceRow.text()).toContain("角色已禁用");

    const statusSelect = wrapper
      .findAll(".admin-toolbar-right select")
      .find((s) => s.findAll("option").some((o) => o.text() === "已禁用"));
    await statusSelect!.setValue("disabled");
    await flushPromises();

    const rows = wrapper.findAll("tbody tr");
    expect(rows).toHaveLength(2);
    expect(rows.some((row) => row.find(".table-cell-main").text() === "alice")).toBe(true);
    expect(rows.some((row) => row.find(".table-cell-main").text() === "bob")).toBe(true);
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
    const sectionTitles = modal.findAll(".modal-form-section h4").map((item) => item.text());
    expect(sectionTitles).toEqual(["基本信息", "密码", "系统角色"]);
    expect(modal.text()).toContain("昵称");
    expect(modal.text()).toContain("头像 URL");
    expect(modal.text()).toContain("手机号");
    expect(modal.text()).toContain("部门");
    expect(modal.text()).toContain("职位");
    expect(modal.text()).toContain("确认密码");
    expect(modal.find('input[autocomplete="off"]').exists()).toBe(true);
  });

  it("submits detailed profile fields when creating a user", async () => {
    createAdminUser.mockResolvedValue(makeUser({ id: "u-3", username: "carol" }));

    const UsersView = (await import("./UsersView.vue")).default;
    const wrapper = mount(UsersView);

    await flushPromises();
    await wrapper.find(".admin-toolbar-left .btn.btn-primary").trigger("click");

    await wrapper.find('input[name="username"]').setValue("carol");
    await wrapper.find('input[name="display_name"]').setValue("Carol Wang");
    await wrapper.find('input[name="nickname"]').setValue("cw");
    await wrapper.find('input[name="avatar_url"]').setValue("https://example.com/avatar.png");
    await wrapper.find('input[name="email"]').setValue("carol@example.com");
    await wrapper.find('input[name="phone"]').setValue("13800000000");
    await wrapper.find('input[name="department"]').setValue("内核组");
    await wrapper.find('input[name="title"]').setValue("嵌入式工程师");
    await wrapper.find('input[name="password"]').setValue("password123");
    await wrapper.find('input[name="confirm_password"]').setValue("password123");
    await wrapper.find('input[type="checkbox"][value="r-2"]').setValue(true);
    await wrapper.find("form.modal-form").trigger("submit");
    await flushPromises();

    expect(createAdminUser).toHaveBeenCalledWith({
      username: "carol",
      display_name: "Carol Wang",
      nickname: "cw",
      avatar_url: "https://example.com/avatar.png",
      email: "carol@example.com",
      phone: "13800000000",
      department: "内核组",
      title: "嵌入式工程师",
      password: "password123",
      role_ids: ["r-2"],
    });
  });

  it("submits detailed profile fields when editing a user", async () => {
    listAdminUsers.mockResolvedValue({
      users: [
        makeUser({
          id: "u-1",
          username: "alice",
          display_name: "Alice",
          nickname: "ali",
          avatar_url: "https://example.com/alice.png",
          phone: "13900000000",
          department: "系统组",
          title: "平台管理员",
        }),
      ],
    });
    updateAdminUser.mockResolvedValue(makeUser());

    const UsersView = (await import("./UsersView.vue")).default;
    const wrapper = mount(UsersView);

    await flushPromises();
    await wrapper.find('button[title="编辑"]').trigger("click");
    await wrapper.find('input[name="department"]').setValue("平台组");
    await wrapper.find("form.modal-form").trigger("submit");
    await flushPromises();

    expect(updateAdminUser).toHaveBeenCalledWith("u-1", {
      display_name: "Alice",
      email: "alice@example.com",
      nickname: "ali",
      avatar_url: "https://example.com/alice.png",
      phone: "13900000000",
      department: "平台组",
      title: "平台管理员",
      disabled: false,
    });
    expect(resetAdminUserPassword).not.toHaveBeenCalled();
  });

  it("resets the password from the edit-user modal only when both password fields are filled", async () => {
    updateAdminUser.mockResolvedValue(makeUser());

    const UsersView = (await import("./UsersView.vue")).default;
    const wrapper = mount(UsersView);

    await flushPromises();
    await wrapper.find('button[title="编辑"]').trigger("click");
    await wrapper.find('input[name="password"]').setValue("newpass123");
    await wrapper.find('input[name="confirm_password"]').setValue("newpass123");
    await wrapper.find("form.modal-form").trigger("submit");
    await flushPromises();

    expect(updateAdminUser).toHaveBeenCalled();
    expect(resetAdminUserPassword).toHaveBeenCalledWith("u-1", {
      password: "newpass123",
    });
  });

  it("renders assignable roles from the admin roles API in the create-user modal", async () => {
    const UsersView = (await import("./UsersView.vue")).default;
    const wrapper = mount(UsersView);

    await flushPromises();
    await wrapper.find(".admin-toolbar-left .btn.btn-primary").trigger("click");

    const modal = wrapper.find(".modal-overlay");
    expect(listAdminRoles).toHaveBeenCalled();
    expect(modal.text()).toContain("实验室管理员");
    expect(modal.text()).toContain("开发人员");
    expect(modal.find('input[type="checkbox"][value="r-1"]').exists()).toBe(true);
    expect(modal.find('input[type="checkbox"][value="r-2"]').exists()).toBe(true);
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
    const wrapper = mount(UsersView, {
      attachTo: document.body,
    });

    await flushPromises();

    const firstRow = wrapper.findAll("tbody tr")[0];
    expect(firstRow.find(".action-menu").exists()).toBe(false);

    await firstRow.find('button[title="更多"]').trigger("click");
    await flushPromises();

    const menu = document.body.querySelector(".action-menu");
    expect(menu).not.toBeNull();
    const labels = Array.from(menu!.querySelectorAll(".action-menu-item")).map(
      (button) => button.textContent ?? "",
    );
    expect(labels.some((t) => t.includes("编辑用户"))).toBe(true);
    expect(labels.some((t) => t.includes("重置密码"))).toBe(true);

    wrapper.unmount();
  });

  it("deletes users through the REST user resource endpoint wrapper", async () => {
    const UsersView = (await import("./UsersView.vue")).default;
    const wrapper = mount(UsersView, {
      attachTo: document.body,
    });

    await flushPromises();

    const firstRow = wrapper.findAll("tbody tr")[0];
    await firstRow.find('button[title="更多"]').trigger("click");
    await flushPromises();

    const deleteButton = Array.from(
      document.body.querySelectorAll<HTMLButtonElement>(".action-menu-item"),
    ).find((button) => button.textContent?.includes("删除用户"));
    expect(deleteButton).toBeTruthy();

    deleteButton!.click();
    await flushPromises();

    expect(deleteAdminUser).toHaveBeenCalledWith("u-1");
    expect(disableAdminUser).not.toHaveBeenCalled();

    wrapper.unmount();
  });
});

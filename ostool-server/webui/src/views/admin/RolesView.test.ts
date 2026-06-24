import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AdminPermissionResponse, AdminRoleResponse } from "@/types/api";

const listAdminRoles = vi.fn();
const listAdminPermissions = vi.fn();
const createAdminRole = vi.fn();
const updateAdminRole = vi.fn();
const deleteAdminRole = vi.fn();
const routerPush = vi.fn();
const routerReplace = vi.fn();
const routeState = {
  name: "admin-user-roles" as string,
  params: {} as Record<string, string>,
};
const uiStore = {
  setError: vi.fn(),
  setSuccess: vi.fn(),
  confirm: vi.fn(),
};

vi.mock("@/api", () => ({
  api: {
    listAdminRoles,
    listAdminPermissions,
    createAdminRole,
    updateAdminRole,
    deleteAdminRole,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("vue-router", () => ({
  useRoute: () => routeState,
  useRouter: () => ({
    push: routerPush,
    replace: routerReplace,
  }),
}));

function makePermission(overrides: Partial<AdminPermissionResponse> = {}): AdminPermissionResponse {
  return {
    id: overrides.id ?? "p-1",
    code: overrides.code ?? "boards.update",
    name: overrides.name ?? "编辑开发板",
    description: overrides.description ?? "编辑开发板配置",
  };
}

function makeRole(permission = makePermission()): AdminRoleResponse {
  return {
    id: "r-1",
    name: "admin",
    display_name: "管理员",
    description: "平台管理员",
    system: true,
    permissions: [permission],
    created_at: "2026-04-01T00:00:00Z",
    updated_at: "2026-04-01T00:00:00Z",
  };
}

describe("RolesView", () => {
  beforeEach(() => {
    listAdminRoles.mockReset();
    listAdminPermissions.mockReset();
    createAdminRole.mockReset();
    updateAdminRole.mockReset();
    deleteAdminRole.mockReset();
    routerPush.mockReset();
    routerReplace.mockReset();
    routeState.name = "admin-user-roles";
    routeState.params = {};
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);
    const permissions = [
      makePermission({ id: "p-1", code: "boards.read", name: "查看开发板" }),
      makePermission(),
      makePermission({
        id: "p-3",
        code: "leases.delete",
        name: "删除租赁",
        description: "删除租赁记录",
      }),
      makePermission({
        id: "p-4",
        code: "sessions.delete",
        name: "删除会话租约",
        description: "删除会话租约记录",
      }),
    ];
    listAdminRoles.mockResolvedValue({ roles: [makeRole(permissions[0])] });
    listAdminPermissions.mockResolvedValue({ permissions });
  });

  it("renders action buttons on the left and search/filter controls on the right", async () => {
    const RolesView = (await import("./RolesView.vue")).default;
    const wrapper = mount(RolesView);
    await flushPromises();

    expect(wrapper.find(".admin-toolbar-left").text()).toContain("新增角色");
    expect(wrapper.find(".admin-toolbar-right .search-field").exists()).toBe(true);
    expect(wrapper.findAll(".admin-toolbar-right .filter-field").length).toBe(1);
  });

  it("routes to the new role editor when creating a role", async () => {
    const RolesView = (await import("./RolesView.vue")).default;
    const wrapper = mount(RolesView);
    await flushPromises();

    await wrapper.find(".admin-toolbar-left .btn.btn-primary").trigger("click");

    expect(routerPush).toHaveBeenCalledWith({ name: "admin-user-role-new" });
  });

  it("routes to the role editor when editing a role", async () => {
    const RolesView = (await import("./RolesView.vue")).default;
    const wrapper = mount(RolesView);
    await flushPromises();

    await wrapper.find('button[title="编辑"]').trigger("click");

    expect(routerPush).toHaveBeenCalledWith({
      name: "admin-user-role-edit",
      params: { roleId: "r-1" },
    });
  });

  it("renders fine-grained permission groups in the role editor", async () => {
    routeState.name = "admin-user-role-new";

    const RolesView = (await import("./RolesView.vue")).default;
    const wrapper = mount(RolesView);
    await flushPromises();

    expect(wrapper.text()).toContain("开发板管理");
    expect(wrapper.text()).toContain("boards.update");
    expect(wrapper.text()).toContain("租赁情况");
    expect(wrapper.text()).toContain("leases.delete");
    expect(wrapper.text()).toContain("会话租约");
    expect(wrapper.text()).toContain("sessions.delete");
  });

  it("toggles all permissions in a group from the module switch", async () => {
    routeState.name = "admin-user-role-new";

    const RolesView = (await import("./RolesView.vue")).default;
    const wrapper = mount(RolesView);
    await flushPromises();

    const boardGroup = wrapper.findAll(".permission-matrix-row")
      .find((row) => row.text().includes("开发板管理"));
    expect(boardGroup).toBeTruthy();

    const groupToggle = boardGroup!.find(".permission-group-toggle input");
    const permissionCheckboxes = boardGroup!.findAll(".permission-matrix-options input");
    expect(permissionCheckboxes).toHaveLength(2);
    expect(permissionCheckboxes.every((item) => (item.element as HTMLInputElement).checked)).toBe(false);

    await groupToggle.setValue(true);

    expect(permissionCheckboxes.every((item) => (item.element as HTMLInputElement).checked)).toBe(true);

    await groupToggle.setValue(false);

    expect(permissionCheckboxes.every((item) => (item.element as HTMLInputElement).checked)).toBe(false);
  });
});

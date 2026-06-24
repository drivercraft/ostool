import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AdminPermissionResponse, AdminRoleResponse } from "@/types/api";

const listAdminRoles = vi.fn();
const listAdminPermissions = vi.fn();
const createAdminRole = vi.fn();
const updateAdminRole = vi.fn();
const deleteAdminRole = vi.fn();
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

function makePermission(): AdminPermissionResponse {
  return {
    id: "p-1",
    code: "resources.manage",
    name: "资源管理",
    description: "管理开发板资源",
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
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);
    const permission = makePermission();
    listAdminRoles.mockResolvedValue({ roles: [makeRole(permission)] });
    listAdminPermissions.mockResolvedValue({ permissions: [permission] });
  });

  it("renders action buttons on the left and search/filter controls on the right", async () => {
    const RolesView = (await import("./RolesView.vue")).default;
    const wrapper = mount(RolesView);
    await flushPromises();

    expect(wrapper.find(".admin-toolbar-left").text()).toContain("新增角色");
    expect(wrapper.find(".admin-toolbar-right .search-field").exists()).toBe(true);
    expect(wrapper.findAll(".admin-toolbar-right .filter-field").length).toBe(1);
  });
});

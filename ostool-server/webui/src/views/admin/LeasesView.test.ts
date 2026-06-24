import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AdminUserResponse, BoardConfig, LeaseResponse } from "@/types/api";

const listAdminLeases = vi.fn();
const createAdminLease = vi.fn();
const updateAdminLease = vi.fn();
const deleteAdminLease = vi.fn();
const listAdminUsers = vi.fn();
const listBoards = vi.fn();
const uiStore = {
  setError: vi.fn(),
  setSuccess: vi.fn(),
  confirm: vi.fn(),
};

vi.mock("@/api", () => ({
  api: {
    listAdminLeases,
    createAdminLease,
    updateAdminLease,
    deleteAdminLease,
    listAdminUsers,
    listBoards,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

function makeUser(): AdminUserResponse {
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
  };
}

function makeBoard(id = "board-1"): BoardConfig {
  return {
    id,
    board_type: "rk3568",
    tags: [],
    serial: null,
    power_management: { kind: "custom", power_on_cmd: "true", power_off_cmd: "true" },
    boot: { kind: "pxe", notes: null },
    notes: null,
    disabled: false,
  };
}

function makeLease(): LeaseResponse {
  return {
    lease: {
      id: "lease-1",
      user_id: "u-1",
      session_id: "session-1",
      board_id: "board-1",
      board_type: "rk3568",
      required_tags: [],
      state: "active",
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
      expires_at: "2026-01-01T02:00:00Z",
      released_at: null,
      failure_message: null,
    },
    session: {
      id: "session-1",
      board_id: "board-1",
      client_name: "Alice",
      created_at: "2026-01-01T00:00:00Z",
      expires_at: "2026-01-01T02:00:00Z",
      state: "active",
    },
  };
}

describe("LeasesView", () => {
  beforeEach(() => {
    [listAdminLeases, createAdminLease, updateAdminLease, deleteAdminLease, listAdminUsers, listBoards]
      .forEach((fn) => fn.mockReset());
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);
    listAdminLeases.mockResolvedValue({ leases: [makeLease()] });
    listAdminUsers.mockResolvedValue({ users: [makeUser()] });
    listBoards.mockResolvedValue([makeBoard("board-1"), makeBoard("board-2")]);
    createAdminLease.mockResolvedValue(makeLease());
    updateAdminLease.mockResolvedValue(makeLease());
    deleteAdminLease.mockResolvedValue(undefined);
  });

  it("renders create/refresh actions on the left and search/filter controls on the right", async () => {
    const LeasesView = (await import("./LeasesView.vue")).default;
    const wrapper = mount(LeasesView);
    await flushPromises();

    expect(wrapper.find(".admin-toolbar-left").text()).toContain("新增租赁");
    expect(wrapper.find(".admin-toolbar-left").text()).toContain("刷新");
    expect(wrapper.find(".admin-toolbar-right .search-field").exists()).toBe(true);
    expect(wrapper.findAll(".admin-toolbar-right .filter-field").length).toBe(1);
    expect(wrapper.text()).toContain("租赁时间段");
  });

  it("creates an admin lease for a selected user and board", async () => {
    const LeasesView = (await import("./LeasesView.vue")).default;
    const wrapper = mount(LeasesView);
    await flushPromises();

    await wrapper.find(".admin-toolbar-left .btn.btn-primary").trigger("click");
    await flushPromises();

    const modal = wrapper.get(".modal-card");
    await modal.findAll("select")[0].setValue("u-1");
    await modal.findAll("select")[1].setValue("board-2");
    await modal.get('input[type="datetime-local"]').setValue("2026-01-01T03:00");
    await modal.get("form").trigger("submit");
    await flushPromises();

    expect(createAdminLease).toHaveBeenCalledWith({
      user_id: "u-1",
      board_id: "board-2",
      client_name: null,
      expires_at: new Date("2026-01-01T03:00").toISOString(),
    });
  });

  it("updates an active lease and releases it", async () => {
    const LeasesView = (await import("./LeasesView.vue")).default;
    const wrapper = mount(LeasesView);
    await flushPromises();

    await wrapper.get('button[title="编辑"]').trigger("click");
    await flushPromises();

    const modal = wrapper.get(".modal-card");
    await modal.get('input[type="datetime-local"]').setValue("2026-01-01T04:00");
    await modal.get("form").trigger("submit");
    await flushPromises();

    expect(updateAdminLease).toHaveBeenCalledWith("lease-1", {
      expires_at: new Date("2026-01-01T04:00").toISOString(),
      failure_message: null,
    });

    await wrapper.get('button[title="释放"]').trigger("click");
    await flushPromises();

    expect(deleteAdminLease).toHaveBeenCalledWith("lease-1");
  });
});

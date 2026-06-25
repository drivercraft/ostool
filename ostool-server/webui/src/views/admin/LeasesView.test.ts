import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AdminUserResponse, BoardConfig, LeaseResponse } from "@/types/api";

const listAdminLeases = vi.fn();
const createAdminLease = vi.fn();
const updateAdminLease = vi.fn();
const startAdminLeaseSession = vi.fn();
const releaseAdminLease = vi.fn();
const deleteAdminLease = vi.fn();
const listAdminUsers = vi.fn();
const listBoards = vi.fn();
const routerPush = vi.fn();
const uiStore = {
  setError: vi.fn(),
  setSuccess: vi.fn(),
  confirm: vi.fn(),
};
const authStore = {
  hasPermission: vi.fn(),
};

vi.mock("vue-router", async () => {
  const actual = await vi.importActual<typeof import("vue-router")>("vue-router");
  return {
    ...actual,
    useRouter: () => ({ push: routerPush }),
  };
});

vi.mock("@/api", () => ({
  api: {
    listAdminLeases,
    createAdminLease,
    updateAdminLease,
    startAdminLeaseSession,
    releaseAdminLease,
    deleteAdminLease,
    listAdminUsers,
    listBoards,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("@/stores/auth", () => ({
  useAuthStore: () => authStore,
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
      starts_at: "2026-01-01T00:00:00Z",
      expires_at: "2026-01-01T02:00:00Z",
      released_at: null,
      failure_message: null,
    },
    session: {
      id: "session-1",
      board_id: "board-1",
      client_name: "Alice",
      source_ip: null,
      created_at: "2026-01-01T00:00:00Z",
      expires_at: "2026-01-01T02:00:00Z",
      state: "active",
    },
  };
}

function makeLeaseWithHistoricalSession(): LeaseResponse {
  const item = makeLease();
  return {
    lease: {
      ...item.lease,
      state: "released",
      released_at: "2026-01-01T02:00:00Z",
    },
    session: null,
  };
}

describe("LeasesView", () => {
  beforeEach(() => {
    [listAdminLeases, createAdminLease, updateAdminLease, startAdminLeaseSession, releaseAdminLease, deleteAdminLease, listAdminUsers, listBoards]
      .forEach((fn) => fn.mockReset());
    authStore.hasPermission.mockReset();
    authStore.hasPermission.mockReturnValue(true);
    routerPush.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);
    listAdminLeases.mockResolvedValue({ leases: [makeLease()] });
    listAdminUsers.mockResolvedValue({ users: [makeUser()] });
    listBoards.mockResolvedValue([makeBoard("board-1"), makeBoard("board-2")]);
    createAdminLease.mockResolvedValue(makeLease());
    updateAdminLease.mockResolvedValue(makeLease());
    startAdminLeaseSession.mockResolvedValue(makeLease());
    releaseAdminLease.mockResolvedValue(undefined);
    deleteAdminLease.mockResolvedValue(undefined);
  });

  it("renders create action on the left and search/filter controls on the right", async () => {
    const LeasesView = (await import("./LeasesView.vue")).default;
    const wrapper = mount(LeasesView);
    await flushPromises();

    expect(wrapper.find(".admin-toolbar-left").text()).toContain("新增租赁");
    expect(wrapper.find(".admin-toolbar-left").text()).not.toContain("刷新");
    expect(wrapper.find(".admin-toolbar-right .search-field").exists()).toBe(true);
    expect(wrapper.findAll(".admin-toolbar-right .filter-field").length).toBe(1);
    expect(wrapper.text()).toContain("租赁时间段");
    expect(wrapper.find("thead").text()).not.toContain("时长");
    expect(wrapper.text()).toContain("生效中");
    expect(wrapper.text()).toContain("时长 2 小时");
  });

  it("navigates to the standalone lease editor when creating a lease", async () => {
    const LeasesView = (await import("./LeasesView.vue")).default;
    const wrapper = mount(LeasesView);
    await flushPromises();

    await wrapper.find(".admin-toolbar-left .btn.btn-primary").trigger("click");

    expect(routerPush).toHaveBeenCalledWith({ name: "admin-rental-lease-new" });
  });

  it("renders edit, enable/disable, and more row actions", async () => {
    const LeasesView = (await import("./LeasesView.vue")).default;
    const wrapper = mount(LeasesView);
    await flushPromises();

    const firstRow = wrapper.find("tbody tr");
    expect(firstRow.find('button[title="编辑"]').exists()).toBe(true);
    expect(firstRow.find('button[title="禁用"]').exists()).toBe(true);
    expect(firstRow.find('button[title="更多"]').exists()).toBe(true);
  });

  it("navigates to the standalone lease editor and releases from the disable action", async () => {
    const LeasesView = (await import("./LeasesView.vue")).default;
    const wrapper = mount(LeasesView);
    await flushPromises();

    await wrapper.get('button[title="编辑"]').trigger("click");
    await flushPromises();

    expect(routerPush).toHaveBeenCalledWith({
      name: "admin-rental-lease-edit",
      params: { leaseId: "lease-1" },
    });

    await wrapper.get('button[title="禁用"]').trigger("click");
    await flushPromises();

    expect(releaseAdminLease).toHaveBeenCalledWith("lease-1");
  });

  it("opens more actions, navigates to the session, and deletes the lease", async () => {
    const LeasesView = (await import("./LeasesView.vue")).default;
    const wrapper = mount(LeasesView, {
      attachTo: document.body,
    });
    await flushPromises();

    await wrapper.find('button[title="更多"]').trigger("click");
    await flushPromises();

    const menu = document.body.querySelector(".action-menu.action-menu--floating");
    expect(menu).not.toBeNull();
    const items = Array.from(menu!.querySelectorAll(".action-menu-item"));
    expect(items.map((item) => item.textContent ?? "").some((text) => text.includes("转到会话"))).toBe(true);
    expect(items.map((item) => item.textContent ?? "").some((text) => text.includes("删除租赁"))).toBe(true);

    (items[0] as HTMLButtonElement).click();
    await flushPromises();

    expect(routerPush).toHaveBeenCalledWith({
      path: "/admin/rentals/sessions",
      query: { q: "session-1" },
    });

    await wrapper.find('button[title="更多"]').trigger("click");
    await flushPromises();

    const deleteItem = Array.from(
      document.body.querySelectorAll<HTMLButtonElement>(".action-menu.action-menu--floating .action-menu-item"),
    ).find((item) => item.textContent?.includes("删除租赁"));
    expect(deleteItem).toBeTruthy();
    deleteItem!.click();
    await flushPromises();

    expect(deleteAdminLease).toHaveBeenCalledWith("lease-1");
    wrapper.unmount();
  });

  it("disables lease deletion without leases.delete permission", async () => {
    authStore.hasPermission.mockReturnValue(false);

    const LeasesView = (await import("./LeasesView.vue")).default;
    const wrapper = mount(LeasesView, {
      attachTo: document.body,
    });
    await flushPromises();

    await wrapper.find('button[title="更多"]').trigger("click");
    await flushPromises();

    const deleteItem = Array.from(
      document.body.querySelectorAll<HTMLButtonElement>(".action-menu.action-menu--floating .action-menu-item"),
    ).find((item) => item.textContent?.includes("删除租赁"));
    expect(deleteItem).toBeTruthy();
    expect(deleteItem!.disabled).toBe(true);

    wrapper.unmount();
  });

  it("navigates to a historical session when the lease only has session_id", async () => {
    listAdminLeases.mockResolvedValue({ leases: [makeLeaseWithHistoricalSession()] });

    const LeasesView = (await import("./LeasesView.vue")).default;
    const wrapper = mount(LeasesView, {
      attachTo: document.body,
    });
    await flushPromises();

    await wrapper.find('button[title="更多"]').trigger("click");
    await flushPromises();

    const sessionItem = Array.from(
      document.body.querySelectorAll<HTMLButtonElement>(".action-menu.action-menu--floating .action-menu-item"),
    ).find((item) => item.textContent?.includes("转到会话"));
    expect(sessionItem).toBeTruthy();
    expect(sessionItem!.disabled).toBe(false);

    sessionItem!.click();
    await flushPromises();

    expect(routerPush).toHaveBeenCalledWith({
      path: "/admin/rentals/sessions",
      query: { q: "session-1" },
    });

    wrapper.unmount();
  });
});

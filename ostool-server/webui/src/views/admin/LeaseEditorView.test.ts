import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AdminUserResponse, BoardConfig, LeaseResponse } from "@/types/api";

const listAdminLeases = vi.fn();
const createAdminLease = vi.fn();
const updateAdminLease = vi.fn();
const listAdminUsers = vi.fn();
const listBoards = vi.fn();
const routerPush = vi.fn();
const routerReplace = vi.fn();
let routeMock = {
  name: "admin-rental-lease-new",
  params: {} as Record<string, string>,
};
const uiStore = {
  setError: vi.fn(),
  setSuccess: vi.fn(),
};

vi.mock("vue-router", async () => {
  const actual = await vi.importActual<typeof import("vue-router")>("vue-router");
  return {
    ...actual,
    useRoute: () => routeMock,
    useRouter: () => ({ push: routerPush, replace: routerReplace }),
    RouterLink: {
      props: ["to"],
      template: "<a><slot /></a>",
    },
  };
});

vi.mock("@/api", () => ({
  api: {
    admin: {
      listAdminLeases,
      createAdminLease,
      updateAdminLease,
      listAdminUsers,
      listBoards,
    },
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
    status: "active",
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
      session_id: null,
      board_id: "board-1",
      board_type: "rk3568",
      required_tags: [],
      state: "active",
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
      starts_at: "2026-01-01T01:00:00Z",
      expires_at: "2026-01-01T02:00:00Z",
      released_at: null,
      failure_message: null,
    },
    session: null,
  };
}

function makeLongLease(): LeaseResponse {
  return {
    lease: {
      id: "lease-long",
      user_id: "u-1",
      session_id: null,
      board_id: "board-1",
      board_type: "rk3568",
      required_tags: [],
      state: "active",
      created_at: "2026-12-01T00:00:00Z",
      updated_at: "2026-12-01T00:00:00Z",
      starts_at: "2026-12-01T00:00:00Z",
      expires_at: "2027-02-01T00:00:00Z",
      released_at: null,
      failure_message: null,
    },
    session: null,
  };
}

describe("LeaseEditorView", () => {
  beforeEach(() => {
    routeMock = {
      name: "admin-rental-lease-new",
      params: {},
    };
    [listAdminLeases, createAdminLease, updateAdminLease, listAdminUsers, listBoards, routerPush, routerReplace, uiStore.setError, uiStore.setSuccess]
      .forEach((fn) => fn.mockReset());
    listAdminLeases.mockResolvedValue({ leases: [makeLease(), makeLongLease()] });
    listAdminUsers.mockResolvedValue({ users: [makeUser()] });
    listBoards.mockResolvedValue([makeBoard("board-1"), makeBoard("board-2")]);
    createAdminLease.mockResolvedValue(makeLease());
    updateAdminLease.mockResolvedValue(makeLease());
  });

  it("renders a standalone create page with calendar and configuration panels", async () => {
    const LeaseEditorView = (await import("./LeaseEditorView.vue")).default;
    const wrapper = mount(LeaseEditorView);
    await flushPromises();

    expect(wrapper.text()).toContain("新增租赁");
    expect(wrapper.text()).toContain("预约占用情况");
    expect(wrapper.text()).toContain("租赁配置");
    expect(wrapper.text()).toContain("预约时间");
    expect(wrapper.text()).toContain("时");
    expect(wrapper.text()).toContain("日");
    expect(wrapper.text()).toContain("月");
    expect(wrapper.text()).toContain("年");
    expect(wrapper.findAll("select")[0].text()).toContain("board-1");
    expect(wrapper.findAll("select")[1].text()).toContain("Alice");
    expect(wrapper.find(".lease-calendar-month").exists()).toBe(true);
    await wrapper.findAll('input[type="datetime-local"]')[0].setValue("2026-01-01T00:00");
    await flushPromises();
    await wrapper.findAll(".lease-calendar-tabs button")[0].trigger("click");
    await flushPromises();
    expect(wrapper.find(".lease-calendar-hour").exists()).toBe(true);
    expect(wrapper.findAll(".lease-calendar-cell")).toHaveLength(24);
    expect(wrapper.find(".lease-calendar-event").exists()).toBe(true);

    await wrapper.findAll(".lease-calendar-tabs button")[1].trigger("click");
    await flushPromises();
    expect(wrapper.find(".lease-calendar-day").exists()).toBe(true);
    expect(wrapper.findAll(".lease-calendar-cell")).toHaveLength(35);
    expect(wrapper.find(".lease-calendar-event").exists()).toBe(true);

    await wrapper.findAll(".lease-calendar-tabs button")[2].trigger("click");
    await flushPromises();
    expect(wrapper.find(".lease-calendar-month").exists()).toBe(true);

    await wrapper.findAll(".lease-calendar-tabs button")[3].trigger("click");
    await flushPromises();
    expect(wrapper.find(".lease-calendar-year").exists()).toBe(true);

    await wrapper.findAll('input[type="datetime-local"]')[0].setValue("2027-01-15T00:00");
    await wrapper.find(".lease-calendar-nav .btn").trigger("click");
    await flushPromises();
    await wrapper.findAll(".lease-calendar-tabs button")[2].trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("整月占用");
  });

  it("creates a lease from selected user, board, and time window", async () => {
    const LeaseEditorView = (await import("./LeaseEditorView.vue")).default;
    const wrapper = mount(LeaseEditorView);
    await flushPromises();

    await wrapper.findAll("select")[0].setValue("board-2");
    await wrapper.findAll("select")[1].setValue("u-1");
    await wrapper.find('input[placeholder="例如 手动分配给 Alice"]').setValue("预约调试");
    const dateInputs = wrapper.findAll('input[type="datetime-local"]');
    await dateInputs[0].setValue("2026-01-01T03:00");
    await dateInputs[1].setValue("2026-01-01T04:00");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(createAdminLease).toHaveBeenCalledWith({
      user_id: "u-1",
      board_id: "board-2",
      client_name: "预约调试",
      starts_at: new Date("2026-01-01T03:00").toISOString(),
      expires_at: new Date("2026-01-01T04:00").toISOString(),
    });
    expect(routerPush).toHaveBeenCalledWith({ name: "admin-rental-leases" });
  });

  it("edits an existing lease in the same calendar editor page", async () => {
    routeMock = {
      name: "admin-rental-lease-edit",
      params: { leaseId: "lease-1" },
    };

    const LeaseEditorView = (await import("./LeaseEditorView.vue")).default;
    const wrapper = mount(LeaseEditorView);
    await flushPromises();

    expect(wrapper.text()).toContain("编辑租赁");
    expect((wrapper.findAll("select")[0].element as HTMLSelectElement).disabled).toBe(true);
    expect((wrapper.findAll("select")[1].element as HTMLSelectElement).disabled).toBe(true);

    const dateInputs = wrapper.findAll('input[type="datetime-local"]');
    await dateInputs[0].setValue("2026-01-01T03:00");
    await dateInputs[1].setValue("2026-01-01T04:00");
    await wrapper.find('input[placeholder="可选"]').setValue("调整时间段");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(updateAdminLease).toHaveBeenCalledWith("lease-1", {
      starts_at: new Date("2026-01-01T03:00").toISOString(),
      expires_at: new Date("2026-01-01T04:00").toISOString(),
      failure_message: "调整时间段",
    });
    expect(routerPush).toHaveBeenCalledWith({ name: "admin-rental-leases" });
  });
});

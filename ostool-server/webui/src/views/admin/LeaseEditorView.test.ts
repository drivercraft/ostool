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

function makeOtherUser(): AdminUserResponse {
  return {
    ...makeUser(),
    id: "u-2",
    username: "bob",
    display_name: "Bob",
    email: "bob@example.com",
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

function makeExpiredOtherLease(): LeaseResponse {
  return {
    lease: {
      id: "lease-expired-other",
      user_id: "u-2",
      session_id: null,
      board_id: "board-1",
      board_type: "rk3568",
      required_tags: [],
      state: "expired",
      created_at: "2025-12-01T00:00:00Z",
      updated_at: "2025-12-02T00:00:00Z",
      starts_at: "2025-12-01T00:00:00Z",
      expires_at: "2025-12-02T00:00:00Z",
      released_at: null,
      failure_message: null,
    },
    session: null,
  };
}

function datetimeLocalAfter(hours: number) {
  const date = new Date(Date.now() + hours * 60 * 60 * 1000);
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60000);
  return local.toISOString().slice(0, 16);
}

function formatDateTimeForTest(value: string) {
  return new Date(value).toLocaleString("zh-CN", { hour12: false });
}

describe("LeaseEditorView", () => {
  beforeEach(() => {
    routeMock = {
      name: "admin-rental-lease-new",
      params: {},
    };
    [listAdminLeases, createAdminLease, updateAdminLease, listAdminUsers, listBoards, routerPush, routerReplace, uiStore.setError, uiStore.setSuccess]
      .forEach((fn) => fn.mockReset());
    listAdminLeases.mockResolvedValue({ leases: [makeExpiredOtherLease(), makeLease(), makeLongLease()] });
    listAdminUsers.mockResolvedValue({ users: [makeUser(), makeOtherUser()] });
    listBoards.mockResolvedValue([makeBoard("board-1"), makeBoard("board-2")]);
    createAdminLease.mockResolvedValue(makeLease());
    updateAdminLease.mockResolvedValue(makeLease());
  });

  it("renders a standalone create page with calendar and configuration panels", async () => {
    const LeaseEditorView = (await import("./LeaseEditorView.vue")).default;
    const wrapper = mount(LeaseEditorView);
    await flushPromises();

    expect(wrapper.text()).not.toContain("返回列表");
    expect(wrapper.text()).toContain("取消");
    expect(wrapper.text()).toContain("预约占用情况");
    expect(wrapper.text()).toContain("新增租赁");
    expect(wrapper.text()).not.toContain("预约时间");
    expect(wrapper.text()).toContain("时");
    expect(wrapper.text()).toContain("日");
    expect(wrapper.text()).toContain("月");
    expect(wrapper.text()).toContain("年");
    expect(wrapper.findAll("select")[0].text()).toContain("board-1");
    expect(wrapper.findAll("select")[1].text()).toContain("Alice");
    expect(wrapper.find(".lease-calendar-hour").exists()).toBe(true);
    const now = new Date();
    const hourStart = new Date(now);
    hourStart.setMinutes(0, 0, 0);
    hourStart.setHours(hourStart.getHours() - 12);
    const hourEnd = new Date(hourStart);
    hourEnd.setHours(hourStart.getHours() + 24);
    const currentRangeStart = formatDateTimeForTest(hourStart.toISOString());
    const currentRangeEnd = formatDateTimeForTest(hourEnd.toISOString());
    expect(wrapper.text()).toContain(`${currentRangeStart} ~ ${currentRangeEnd}`);
    const dateInputs = wrapper.findAll('input[type="datetime-local"]');
    expect((dateInputs[0].element as HTMLInputElement).value).toBe("");
    expect((dateInputs[1].element as HTMLInputElement).value).toBe("");
    expect(wrapper.text()).toContain("已有租赁");
    expect(wrapper.text()).toContain("3 条");
    expect(wrapper.find(".lease-calendar-panel .lease-calendar-reservations").exists()).toBe(false);
    expect(wrapper.find(".lease-config-panel .lease-calendar-reservations").exists()).toBe(true);
    const sectionTitles = wrapper.findAll(".lease-config-panel .form-section-header h4").map((item) => item.text());
    expect(sectionTitles).toEqual(["新增租赁", "已有租赁"]);
    expect(wrapper.findAll(".lease-calendar-reservation")).toHaveLength(3);
    expect(wrapper.text()).toContain("2026/12/1");
    expect(wrapper.text()).toContain("Alice / alice");
    expect(wrapper.text()).toContain("Bob / bob");
    expect(wrapper.text()).toContain("已过期");
    const bobReservation = wrapper.findAll(".lease-calendar-reservation").find((item) => item.text().includes("Bob / bob"));
    expect(bobReservation).toBeTruthy();
    await bobReservation!.trigger("click");
    await flushPromises();
    expect(wrapper.find(".lease-calendar-event").text()).toContain("Bob / bob");
    expect(wrapper.find(".lease-calendar-event").classes()).not.toContain("is-conflicting");

    const longReservation = wrapper.findAll(".lease-calendar-reservation").find((item) => item.text().includes("2026/12/1"));
    expect(longReservation).toBeTruthy();
    await longReservation!.trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("2026/12/1");
    await dateInputs[0].setValue("2026-01-01T01:00");
    await dateInputs[1].setValue("2026-01-01T02:00");
    await flushPromises();
    await wrapper.findAll(".lease-calendar-tabs button")[0].trigger("click");
    await flushPromises();
    expect(wrapper.find(".lease-calendar-hour").exists()).toBe(true);
    expect(wrapper.findAll(".lease-calendar-cell")).toHaveLength(24);
    expect(wrapper.find(".lease-calendar-event").exists()).toBe(true);
    const disabledCell = wrapper.find(".lease-calendar-cell:disabled");
    expect(disabledCell.exists()).toBe(true);
    expect(disabledCell.find(".lease-calendar-status-icon.is-unavailable").exists()).toBe(true);
    await disabledCell.trigger("click");
    await flushPromises();
    expect(disabledCell.classes()).not.toContain("is-selected");

    await dateInputs[0].setValue(datetimeLocalAfter(-48));
    await dateInputs[1].setValue(datetimeLocalAfter(-47));
    await flushPromises();
    expect(wrapper.findAll(".lease-calendar-cell:disabled")).toHaveLength(24);

    await dateInputs[0].setValue(datetimeLocalAfter(48));
    await dateInputs[1].setValue(datetimeLocalAfter(49));
    await flushPromises();
    await dateInputs[0].setValue("");
    await dateInputs[1].setValue("");
    await flushPromises();
    const selectableCells = wrapper.findAll(".lease-calendar-cell:not(:disabled)");
    await selectableCells[0].trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".lease-calendar-cell.is-selected")).toHaveLength(1);
    expect(wrapper.find(".lease-calendar-status-icon.is-selected").exists()).toBe(true);
    expect((dateInputs[0].element as HTMLInputElement).value).toBeTruthy();
    expect((dateInputs[1].element as HTMLInputElement).value).toBeTruthy();
    await selectableCells[0].trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".lease-calendar-cell.is-selected")).toHaveLength(0);

    await dateInputs[0].setValue("2026-01-01T01:00");
    await dateInputs[1].setValue("2026-01-01T02:00");
    await flushPromises();
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
    const startsAt = datetimeLocalAfter(72);
    const expiresAt = datetimeLocalAfter(73);
    await dateInputs[0].setValue(startsAt);
    await dateInputs[1].setValue(expiresAt);
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(createAdminLease).toHaveBeenCalledWith({
      user_id: "u-1",
      board_id: "board-2",
      client_name: "预约调试",
      starts_at: new Date(startsAt).toISOString(),
      expires_at: new Date(expiresAt).toISOString(),
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

    expect(wrapper.text()).not.toContain("返回列表");
    expect(wrapper.text()).toContain("取消");
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

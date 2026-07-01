import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AdminSessionResponse, AdminUserResponse, BoardConfig, SessionRecord } from "@/types/api";

const listBoards = vi.fn();
const listSessions = vi.fn();
const listAdminUsers = vi.fn();
const deleteSession = vi.fn();
const updateSession = vi.fn();
const closeSession = vi.fn();
const routerPush = vi.fn();
const route = {
  query: {} as Record<string, string>,
};
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
  confirm: vi.fn(),
};
const authStore = {
  hasPermission: vi.fn(),
};

vi.mock("@/api", () => ({
  api: {
    admin: {
      listBoards,
      listSessions,
      listAdminUsers,
      deleteSession,
      updateSession,
      closeSession,
    },
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("@/stores/auth", () => ({
  useAuthStore: () => authStore,
}));

vi.mock("vue-router", () => ({
  useRoute: () => route,
  useRouter: () => ({ push: routerPush }),
}));

function makeBoard(id = "orangepi5plus-1"): BoardConfig {
  return {
    id,
    board_type: "orangepi5plus",
    tags: [],
    serial: null,
    power_management: {
      kind: "custom",
      power_on_cmd: "echo on",
      power_off_cmd: "echo off",
    },
    boot: {
      kind: "uboot",
      use_tftp: true,
      dtb_name: null,
      kernel_load_addr: null,
      fit_load_addr: null,
      bootm_addr: null,
      network_mode: "dhcp",
      board_ip: null,
      server_ip: null,
      netmask: null,
      gatewayip: null,
    },
    notes: null,
    disabled: false,
  };
}

function makeSession(overrides: Partial<SessionRecord> = {}): SessionRecord {
  return {
    id: "session-1",
    board_id: "orangepi5plus-1",
    client_name: "web-ui",
    source_ip: "192.168.1.10",
    state: "active",
    created_at: "2026-04-08T00:00:00Z",
    last_heartbeat_at: "2026-04-08T00:00:00Z",
    expires_at: "2026-04-08T00:05:00Z",
    ended_at: null,
    failure_message: null,
    ...overrides,
  };
}

function makeAdminSession(overrides: Partial<SessionRecord> = {}): AdminSessionResponse {
  const session = makeSession(overrides);
  return {
    session,
    lease: {
      id: "lease-1",
      user_id: "user-1",
      session_id: session.id,
      board_id: session.board_id,
      board_type: "orangepi5plus",
      required_tags: [],
      state: session.state === "released" || session.state === "expired" ? "released" : "active",
      created_at: "2026-04-08T00:00:00Z",
      updated_at: "2026-04-08T00:00:00Z",
      starts_at: "2026-04-08T00:00:00Z",
      expires_at: "2026-04-08T00:05:00Z",
      released_at: null,
      failure_message: null,
    },
    user_id: "user-1",
    source_ip: "192.168.1.10",
  };
}

function makeUser(): AdminUserResponse {
  return {
    id: "user-1",
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
    created_at: "2026-04-08T00:00:00Z",
    updated_at: "2026-04-08T00:00:00Z",
  };
}

describe("SessionsView", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  beforeEach(() => {
    listBoards.mockReset();
    listSessions.mockReset();
    listAdminUsers.mockReset();
    deleteSession.mockReset();
    updateSession.mockReset();
    closeSession.mockReset();
    routerPush.mockReset();
    authStore.hasPermission.mockReset();
    authStore.hasPermission.mockReturnValue(true);
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);
    route.query = {};
    listBoards.mockResolvedValue([makeBoard()]);
    listAdminUsers.mockResolvedValue({ users: [makeUser()] });
    listSessions.mockResolvedValue({ sessions: [makeAdminSession()] });
    updateSession.mockResolvedValue(makeAdminSession({ client_name: "updated-client" }));
    closeSession.mockResolvedValue(undefined);
  });

  it("deletes session records and refreshes the list", async () => {
    deleteSession.mockResolvedValue(undefined);
    listSessions
      .mockResolvedValueOnce({ sessions: [makeAdminSession()] })
      .mockResolvedValueOnce({ sessions: [makeAdminSession({ state: "releasing" })] });

    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    await wrapper.find('button[title="更多"]').trigger("click");
    await flushPromises();
    const deleteButton = document.body.querySelectorAll<HTMLButtonElement>(".action-menu-item")[1];
    deleteButton.click();
    await flushPromises();

    expect(deleteSession).toHaveBeenCalledWith("session-1");
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已删除会话记录 session-1");
    expect(listSessions).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).toContain("断开中");
  });

  it("renders refresh actions on the left and search/filter controls on the right", async () => {
    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    expect(wrapper.find(".admin-toolbar-left").text()).toContain("刷新");
    expect(wrapper.find(".admin-toolbar-right .search-field").exists()).toBe(true);
    expect(wrapper.findAll(".admin-toolbar-right .filter-field").length).toBe(1);
  });

  it("renders session lease columns and a delete row action", async () => {
    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    const headers = wrapper.findAll("th").map((header) => header.text());
    expect(headers).toEqual([
      "序号",
      "会话 ID",
      "源 IP",
      "用户",
      "开发板",
      "客户端",
      "开始时间",
      "剩余/结束时间",
      "状态",
      "操作",
    ]);
    expect(wrapper.text()).toContain("192.168.1.10");
    expect(wrapper.text()).toContain("Alice");
    expect(wrapper.find(".col-actions .row-actions button[title=\"编辑\"]").exists()).toBe(true);
    expect(wrapper.find(".col-actions .row-actions button[title=\"关闭\"]").exists()).toBe(true);
    expect(wrapper.find(".col-actions .row-actions button[title=\"更多\"]").exists()).toBe(true);
  });

  it("keeps the table header visible when there are no sessions", async () => {
    listSessions.mockResolvedValue({ sessions: [] });

    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    expect(wrapper.find("table.data-table thead").exists()).toBe(true);
    expect(wrapper.findAll("tbody tr")).toHaveLength(0);
    expect(wrapper.text()).toContain("会话 ID");
    expect(wrapper.text()).toContain("源 IP");
  });

  it("initializes search from the route query", async () => {
    route.query = { q: "session-1" };

    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    expect((wrapper.find(".search-field input").element as HTMLInputElement).value).toBe("session-1");
  });

  it("disables the close button for releasing sessions", async () => {
    listSessions.mockResolvedValue({ sessions: [makeAdminSession({ state: "releasing" })] });

    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    const closeButton = wrapper.find('button[title="关闭"]');
    expect((closeButton.element as HTMLButtonElement).disabled).toBe(true);
    expect(wrapper.text()).toContain("断开中");
  });

  it("renders released session history and leaves record deletion in the menu", async () => {
    listSessions.mockResolvedValue({
      sessions: [makeAdminSession({ state: "released", ended_at: "2026-04-08T00:04:00Z" })],
    });

    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    expect(wrapper.text()).toContain("已断开");
    await wrapper.find('button[title="更多"]').trigger("click");
    await flushPromises();
    const deleteButton = document.body.querySelectorAll<HTMLButtonElement>(".action-menu-item")[1];
    expect(deleteButton.disabled).toBe(false);
  });

  it("disables session deletion without sessions.delete permission", async () => {
    authStore.hasPermission.mockImplementation((permission: string) => permission === "sessions.update");

    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    expect((wrapper.find('button[title="关闭"]').element as HTMLButtonElement).disabled).toBe(true);
    await wrapper.find('button[title="更多"]').trigger("click");
    await flushPromises();
    const deleteButton = document.body.querySelectorAll<HTMLButtonElement>(".action-menu-item")[1];
    expect(deleteButton.disabled).toBe(true);
  });

  it("edits session records", async () => {
    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    await wrapper.find('button[title="编辑"]').trigger("click");
    await flushPromises();
    const clientInput = wrapper.find<HTMLInputElement>('input[placeholder="客户端名称"]');
    await clientInput.setValue("updated-client");
    await wrapper.find("form.modal-form").trigger("submit");
    await flushPromises();

    expect(updateSession).toHaveBeenCalledWith("session-1", {
      client_name: "updated-client",
      failure_message: null,
    });
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已保存会话记录 session-1");
  });

  it("closes active sessions", async () => {
    listSessions
      .mockResolvedValueOnce({ sessions: [makeAdminSession()] })
      .mockResolvedValueOnce({ sessions: [makeAdminSession({ state: "releasing" })] });

    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    await wrapper.find('button[title="关闭"]').trigger("click");
    await flushPromises();

    expect(closeSession).toHaveBeenCalledWith("session-1");
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已发起关闭会话 session-1");
    expect(listSessions).toHaveBeenCalledTimes(2);
  });

  it("navigates to the related lease from the more menu", async () => {
    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    await wrapper.find('button[title="更多"]').trigger("click");
    await flushPromises();
    const leaseButton = document.body.querySelectorAll<HTMLButtonElement>(".action-menu-item")[0];
    leaseButton.click();
    await flushPromises();

    expect(routerPush).toHaveBeenCalledWith({
      name: "admin-rental-lease-edit",
      params: { leaseId: "lease-1" },
    });
  });
});

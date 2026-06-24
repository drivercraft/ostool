import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BoardConfig, Session } from "@/types/api";

const listBoards = vi.fn();
const listSessions = vi.fn();
const deleteSession = vi.fn();
const route = {
  query: {} as Record<string, string>,
};
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
  confirm: vi.fn(),
};

vi.mock("@/api", () => ({
  api: {
    listBoards,
    listSessions,
    deleteSession,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("vue-router", () => ({
  useRoute: () => route,
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

function makeSession(overrides: Partial<Session> = {}): Session {
  return {
    id: "session-1",
    board_id: "orangepi5plus-1",
    client_name: "web-ui",
    created_at: "2026-04-08T00:00:00Z",
    expires_at: "2026-04-08T00:05:00Z",
    state: "active",
    ...overrides,
  };
}

describe("SessionsView", () => {
  beforeEach(() => {
    listBoards.mockReset();
    listSessions.mockReset();
    deleteSession.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);
    route.query = {};
    listBoards.mockResolvedValue([makeBoard()]);
    listSessions.mockResolvedValue({ sessions: [makeSession()] });
  });

  it("accepts force release responses without throwing and refreshes the list", async () => {
    deleteSession.mockResolvedValue(undefined);
    listSessions
      .mockResolvedValueOnce({ sessions: [makeSession()] })
      .mockResolvedValueOnce({ sessions: [makeSession({ state: "releasing" })] });

    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    const releaseButton = wrapper.findAll("button").find((button) => button.text() === "强制释放");
    await releaseButton!.trigger("click");
    await flushPromises();

    expect(deleteSession).toHaveBeenCalledWith("session-1");
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已发起释放会话 session-1");
    expect(listSessions).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).toContain("释放中");
  });

  it("renders refresh actions on the left and search/filter controls on the right", async () => {
    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    expect(wrapper.find(".admin-toolbar-left").text()).toContain("刷新");
    expect(wrapper.find(".admin-toolbar-right .search-field").exists()).toBe(true);
    expect(wrapper.findAll(".admin-toolbar-right .filter-field").length).toBe(1);
  });

  it("initializes search from the route query", async () => {
    route.query = { q: "session-1" };

    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    expect((wrapper.find(".search-field input").element as HTMLInputElement).value).toBe("session-1");
  });

  it("disables the force release button for releasing sessions", async () => {
    listSessions.mockResolvedValue({ sessions: [makeSession({ state: "releasing" })] });

    const SessionsView = (await import("./SessionsView.vue")).default;
    const wrapper = mount(SessionsView);
    await flushPromises();

    const releaseButton = wrapper.findAll("button").find((button) => button.text() === "强制释放");
    expect((releaseButton!.element as HTMLButtonElement).disabled).toBe(true);
    expect(wrapper.text()).toContain("释放中");
  });
});

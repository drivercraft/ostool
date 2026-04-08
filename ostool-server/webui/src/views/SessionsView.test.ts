import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BoardConfig, Session } from "@/types/api";

const listBoards = vi.fn();
const listSessions = vi.fn();
const deleteSession = vi.fn();
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
};

vi.stubGlobal("confirm", vi.fn(() => true));

vi.mock("@/api/client", () => ({
  api: {
    listBoards,
    listSessions,
    deleteSession,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
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

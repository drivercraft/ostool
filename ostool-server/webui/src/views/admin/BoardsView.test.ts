import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BoardConfig } from "@/types/api";

const listBoards = vi.fn();
const listSessions = vi.fn();
const deleteBoard = vi.fn();
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
    deleteBoard,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

function makeBoard(id = "rk3568-1"): BoardConfig {
  return {
    id,
    board_type: "rk3568",
    tags: ["lab"],
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

describe("BoardsView", () => {
  beforeEach(() => {
    listBoards.mockReset();
    listSessions.mockReset();
    deleteBoard.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);
    listBoards.mockResolvedValue([makeBoard()]);
    listSessions.mockResolvedValue({ sessions: [] });
  });

  it("links board edit actions to the registered edit route", async () => {
    const BoardsView = (await import("./BoardsView.vue")).default;
    const wrapper = mount(BoardsView, {
      global: {
        stubs: {
          RouterLink: {
            name: "RouterLink",
            props: ["to"],
            template: "<a><slot /></a>",
          },
        },
      },
    });
    await flushPromises();

    const editLink = wrapper.findAllComponents({ name: "RouterLink" }).find((link) => {
      return link.text() === "编辑";
    });

    expect(editLink?.props("to")).toEqual({
      name: "admin-resource-board-edit",
      params: { boardId: "rk3568-1" },
    });
  });
});

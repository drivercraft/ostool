import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BoardConfig, LeaseResponse, Session } from "@/types/api";

const listBoards = vi.fn();
const listSessions = vi.fn();
const listAdminLeases = vi.fn();
const updateBoard = vi.fn();
const deleteBoard = vi.fn();
const routerPush = vi.fn();
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
  confirm: vi.fn(),
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
    listBoards,
    listSessions,
    listAdminLeases,
    updateBoard,
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

function makeSession(boardId = "rk3568-1"): Session {
  return {
    id: "session-1",
    board_id: boardId,
    client_name: "web-ui",
    created_at: "2026-04-08T00:00:00Z",
    expires_at: "2026-04-08T00:05:00Z",
    state: "active",
  };
}

function makeLease(boardId = "rk3568-1"): LeaseResponse {
  return {
    lease: {
      id: "lease-1",
      user_id: "user-1",
      session_id: "session-1",
      board_id: boardId,
      board_type: "rk3568",
      required_tags: [],
      state: "active",
      created_at: "2026-04-08T00:00:00Z",
      updated_at: "2026-04-08T00:00:00Z",
      starts_at: "2026-04-08T00:00:00Z",
      expires_at: "2026-04-08T00:05:00Z",
      released_at: null,
      failure_message: null,
    },
    session: makeSession(boardId),
  };
}

describe("BoardsView", () => {
  beforeEach(() => {
    listBoards.mockReset();
    listSessions.mockReset();
    listAdminLeases.mockReset();
    updateBoard.mockReset();
    deleteBoard.mockReset();
    routerPush.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);
    listBoards.mockResolvedValue([makeBoard()]);
    listSessions.mockResolvedValue({ sessions: [] });
    listAdminLeases.mockResolvedValue({ leases: [] });
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
      return link.attributes("title") === "编辑";
    });

    expect(editLink?.props("to")).toEqual({
      name: "admin-resource-board-edit",
      params: { boardId: "rk3568-1" },
    });
  });

  it("renders action buttons on the left and search before filters on the right", async () => {
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

    expect(wrapper.find(".admin-toolbar-left").text()).toContain("新增开发板");
    const rightControls = wrapper.find(".admin-toolbar-right").element.children;
    expect(rightControls[0].classList.contains("search-field")).toBe(true);
    expect(rightControls[1].classList.contains("filter-field")).toBe(true);
    expect(rightControls[2].classList.contains("filter-field")).toBe(true);
  });

  it("renders edit, enable/disable, and more row actions", async () => {
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

    const firstRow = wrapper.find("tbody tr");
    expect(firstRow.find('a[title="编辑"]').exists()).toBe(true);
    expect(firstRow.find('button[title="禁用"]').exists()).toBe(true);
    expect(firstRow.find('button[title="更多"]').exists()).toBe(true);
  });

  it("toggles board disabled state from the row action", async () => {
    const updated = { ...makeBoard(), disabled: true };
    updateBoard.mockResolvedValue(updated);

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

    await wrapper.find('button[title="禁用"]').trigger("click");
    await flushPromises();

    expect(updateBoard).toHaveBeenCalledWith(
      "rk3568-1",
      expect.objectContaining({
        id: "rk3568-1",
        disabled: true,
      }),
    );
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已禁用开发板 rk3568-1");
  });

  it("opens more actions and navigates to the active board lease", async () => {
    listSessions.mockResolvedValue({ sessions: [makeSession()] });
    listAdminLeases.mockResolvedValue({ leases: [makeLease()] });

    const BoardsView = (await import("./BoardsView.vue")).default;
    const wrapper = mount(BoardsView, {
      attachTo: document.body,
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

    await wrapper.find('button[title="更多"]').trigger("click");
    await flushPromises();

    expect(wrapper.find(".table-scroll .action-menu").exists()).toBe(false);
    const menu = document.body.querySelector(".action-menu.action-menu--floating");
    expect(menu).not.toBeNull();
    const items = Array.from(menu!.querySelectorAll(".action-menu-item"));
    expect(items.map((item) => item.textContent ?? "").some((text) => text.includes("转到租赁"))).toBe(true);
    (items[0] as HTMLButtonElement).click();
    await flushPromises();

    expect(routerPush).toHaveBeenCalledWith({
      path: "/admin/rentals/leases",
      query: { q: "lease-1" },
    });

    wrapper.unmount();
  });

  it("closes the floating action menu when clicking outside", async () => {
    const BoardsView = (await import("./BoardsView.vue")).default;
    const wrapper = mount(BoardsView, {
      attachTo: document.body,
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

    await wrapper.find('button[title="更多"]').trigger("click");
    await flushPromises();
    expect(document.body.querySelector(".action-menu.action-menu--floating")).not.toBeNull();

    document.body.click();
    await flushPromises();

    expect(document.body.querySelector(".action-menu.action-menu--floating")).toBeNull();
    wrapper.unmount();
  });
});

import { defineComponent } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BoardEditorDocument } from "@/types/api";

const route = {
  params: {} as Record<string, string>,
};

const push = vi.fn();
const getNewBoardEditor = vi.fn();
const getBoardEditor = vi.fn();
const createBoard = vi.fn();
const updateBoard = vi.fn();
const deleteBoard = vi.fn();
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
};

vi.mock("vue-router", () => ({
  useRoute: () => route,
  useRouter: () => ({ push }),
}));

vi.mock("@/api/client", () => ({
  api: {
    getNewBoardEditor,
    getBoardEditor,
    createBoard,
    updateBoard,
    deleteBoard,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("@jsonforms/vue", () => ({
  JsonForms: defineComponent({
    name: "JsonFormsStub",
    props: {
      data: { type: Object, required: true },
      schema: { type: Object, required: true },
      uischema: { type: Object, required: true },
      renderers: { type: Array, required: true },
      validationMode: { type: String, required: false, default: "" },
    },
    emits: ["change"],
    template: "<div class='jsonforms-stub'></div>",
  }),
}));

vi.mock("@jsonforms/vue-vanilla", () => ({
  vanillaRenderers: [],
}));

function makeDocument(id = "demo-board"): BoardEditorDocument {
  return {
    data: {
      id,
      name: `Board ${id}`,
      board_type: "rk3568",
      tags_text: "lab, usb",
      notes: "",
      disabled: false,
      serial_enabled: true,
      serial_port: "/dev/ttyUSB0",
      serial_baud_rate: 115200,
      power_management_enabled: true,
      power_management_kind: "custom",
      power_management_custom: {
        power_on_cmd: "echo on",
        power_off_cmd: "echo off",
      },
      power_management_zhongsheng_relay: {
        serial_port: "/dev/ttyUSB1",
      },
      boot_kind: "uboot",
      uboot: {
        use_tftp: true,
        kernel_load_addr: "",
        fit_load_addr: "",
        success_regex_text: "",
        fail_regex_text: "",
        uboot_cmd_text: "",
        shell_prefix: "",
        shell_init_cmd: "",
        timeout: null,
      },
      pxe: {
        notes: "",
      },
    },
    schema: {
      type: "object",
      properties: {},
    },
  };
}

describe("BoardEditorView", () => {
  beforeEach(() => {
    route.params = {};
    push.mockReset();
    getNewBoardEditor.mockReset();
    getBoardEditor.mockReset();
    createBoard.mockReset();
    updateBoard.mockReset();
    deleteBoard.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
  });

  it("loads the new-board editor document and refreshes as a whole", async () => {
    getNewBoardEditor.mockResolvedValue(makeDocument());

    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    expect(getNewBoardEditor).toHaveBeenCalledTimes(1);
    expect(getBoardEditor).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("刷新");
    expect(wrapper.text()).not.toContain("刷新串口");

    await wrapper.get("button.ghost-button").trigger("click");
    await flushPromises();

    expect(getNewBoardEditor).toHaveBeenCalledTimes(2);
  });

  it("loads the existing board editor document for edit mode", async () => {
    route.params = { boardId: "demo-board" };
    getBoardEditor.mockResolvedValue(makeDocument("demo-board"));

    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    mount(BoardEditorView);
    await flushPromises();

    expect(getBoardEditor).toHaveBeenCalledWith("demo-board");
    expect(getNewBoardEditor).not.toHaveBeenCalled();
  });

  it("saves the wrapper document and routes to the saved board id", async () => {
    route.params = { boardId: "old-board" };
    const initial = makeDocument("old-board");
    const saved = makeDocument("new-board");
    getBoardEditor.mockResolvedValue(initial);
    updateBoard.mockResolvedValue(saved);

    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    await wrapper.get("button.primary-button").trigger("click");
    await flushPromises();

    expect(updateBoard).toHaveBeenCalledWith("old-board", initial);
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已保存开发板 Board new-board");
    expect(push).toHaveBeenCalledWith("/boards/new-board");
  });
});

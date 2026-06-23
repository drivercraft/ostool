import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DtbFileResponse } from "@/types/api";

const listDtbs = vi.fn();
const createDtb = vi.fn();
const updateDtb = vi.fn();
const deleteDtb = vi.fn();
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
};

vi.stubGlobal("confirm", vi.fn(() => true));

vi.mock("@/api/client", () => ({
  api: {
    listDtbs,
    createDtb,
    updateDtb,
    deleteDtb,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

function makeDtb(name = "board.dtb"): DtbFileResponse {
  return {
    name,
    size: 12,
    updated_at: "2026-04-01T00:00:00Z",
    relative_tftp_path_template: `boot/dtb/${name}`,
  };
}

describe("DtbView", () => {
  beforeEach(() => {
    listDtbs.mockReset();
    createDtb.mockReset();
    updateDtb.mockReset();
    deleteDtb.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    listDtbs.mockResolvedValue([makeDtb()]);
  });

  it("loads DTB list and creates a new DTB", async () => {
    createDtb.mockResolvedValue(makeDtb("new-board.dtb"));

    const DtbView = (await import("./DtbView.vue")).default;
    const wrapper = mount(DtbView);
    await flushPromises();

    expect(listDtbs).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain("board.dtb");

    await wrapper.get('input[placeholder="例如 board.dtb"]').setValue("new-board.dtb");
    const fileInput = wrapper.get('input[type="file"]');
    Object.defineProperty(fileInput.element, "files", {
      value: [new File(["dtb"], "new-board.dtb", { type: "application/octet-stream" })],
      configurable: true,
    });
    await fileInput.trigger("change");

    const uploadButton = wrapper.findAll("button").find((button) => button.text() === "上传 DTB");
    await uploadButton!.trigger("click");
    await flushPromises();

    expect(createDtb).toHaveBeenCalledWith("new-board.dtb", expect.any(File));
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已上传 DTB new-board.dtb");
  });

  it("fills DTB name automatically after choosing a file", async () => {
    const DtbView = (await import("./DtbView.vue")).default;
    const wrapper = mount(DtbView);
    await flushPromises();

    const nameInput = wrapper.get('input[placeholder="例如 board.dtb"]');
    const fileInput = wrapper.get('input[type="file"]');
    Object.defineProperty(fileInput.element, "files", {
      value: [new File(["dtb"], "auto-name.dtb", { type: "application/octet-stream" })],
      configurable: true,
    });

    await fileInput.trigger("change");

    expect((nameInput.element as HTMLInputElement).value).toBe("auto-name.dtb");
  });

  it("renames and deletes an existing DTB", async () => {
    updateDtb.mockResolvedValue(makeDtb("board-v2.dtb"));
    deleteDtb.mockResolvedValue(undefined);

    const DtbView = (await import("./DtbView.vue")).default;
    const wrapper = mount(DtbView);
    await flushPromises();

    const editButton = wrapper.findAll("button").find((button) => button.text() === "修改");
    await editButton!.trigger("click");
    await flushPromises();

    const modal = wrapper.get(".modal-card");
    const renameInput = modal.get('input[placeholder="例如 board.dtb"]');
    await renameInput.setValue("board-v2.dtb");

    const saveButton = modal.findAll("button").find((button) => button.text() === "保存修改");
    await saveButton!.trigger("click");
    await flushPromises();

    expect(updateDtb).toHaveBeenCalledWith("board.dtb", "board-v2.dtb", null);

    const deleteButton = wrapper.findAll("button").find((button) => button.text() === "删除");
    await deleteButton!.trigger("click");
    await flushPromises();

    expect(deleteDtb).toHaveBeenCalledWith("board.dtb");
  });

  it("fills rename draft automatically after choosing a replacement file", async () => {
    const DtbView = (await import("./DtbView.vue")).default;
    const wrapper = mount(DtbView);
    await flushPromises();

    const editButton = wrapper.findAll("button").find((button) => button.text() === "修改");
    await editButton!.trigger("click");
    await flushPromises();

    const modal = wrapper.get(".modal-card");
    const renameInput = modal.get('input[placeholder="例如 board.dtb"]');
    const replaceFileInput = modal.get('input[type="file"]');
    Object.defineProperty(replaceFileInput.element, "files", {
      value: [new File(["dtb"], "renamed-by-file.dtb", { type: "application/octet-stream" })],
      configurable: true,
    });

    await replaceFileInput.trigger("change");

    expect((renameInput.element as HTMLInputElement).value).toBe("renamed-by-file.dtb");
  });
});

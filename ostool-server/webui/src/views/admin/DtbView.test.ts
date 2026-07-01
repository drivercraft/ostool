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
  confirm: vi.fn(),
};

vi.mock("@/api", () => ({
  api: {
    admin: {
      listDtbs,
      createDtb,
      updateDtb,
      deleteDtb,
    },
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
    boot_architecture: "arm64",
    compatible: "test,board",
    description: "测试开发板 DTB",
    disabled: false,
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
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);
    listDtbs.mockResolvedValue([makeDtb()]);
    createDtb.mockResolvedValue(makeDtb("new-board.dtb"));
    updateDtb.mockResolvedValue(makeDtb("board-v2.dtb"));
    deleteDtb.mockResolvedValue(undefined);
  });

  it("loads DTB list and creates a new DTB", async () => {
    const DtbView = (await import("./DtbView.vue")).default;
    const wrapper = mount(DtbView);
    await flushPromises();

    expect(listDtbs).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain("board.dtb");

    await wrapper.find(".admin-toolbar-left .btn.btn-primary").trigger("click");
    await flushPromises();

    const modal = wrapper.get(".modal-card");
    await modal.get('input[placeholder="例如 rk3568-evb.dtb"]').setValue("new-board.dtb");
    await modal.get('input[placeholder="例如 arm64 / riscv64"]').setValue("arm64");
    await modal.get('input[placeholder="例如 rockchip,rk3568-evb"]').setValue("demo,new-board");
    await modal.get("textarea").setValue("新开发板 DTB");
    const fileInput = modal.get('input[type="file"]');
    Object.defineProperty(fileInput.element, "files", {
      value: [new File(["dtb"], "new-board.dtb", { type: "application/octet-stream" })],
      configurable: true,
    });
    await fileInput.trigger("change");

    await modal.get("form").trigger("submit");
    await flushPromises();

    expect(createDtb).toHaveBeenCalledWith("new-board.dtb", expect.any(File), {
      boot_architecture: "arm64",
      compatible: "demo,new-board",
      description: "新开发板 DTB",
    });
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已上传 DTB new-board.dtb");
  });

  it("renders upload action on the left and search/filter controls on the right", async () => {
    const DtbView = (await import("./DtbView.vue")).default;
    const wrapper = mount(DtbView);
    await flushPromises();

    expect(wrapper.find(".admin-toolbar-left").text()).toContain("上传 DTB");
    expect(wrapper.find(".admin-toolbar-left").text()).not.toContain("刷新");
    expect(wrapper.find(".admin-toolbar-right .search-field").exists()).toBe(true);
    expect(wrapper.findAll(".admin-toolbar-right .filter-field").length).toBe(1);
  });

  it("fills DTB name automatically after choosing a file", async () => {
    const DtbView = (await import("./DtbView.vue")).default;
    const wrapper = mount(DtbView);
    await flushPromises();

    await wrapper.find(".admin-toolbar-left .btn.btn-primary").trigger("click");
    await flushPromises();

    const modal = wrapper.get(".modal-card");
    const nameInput = modal.get('input[placeholder="例如 rk3568-evb.dtb"]');
    const fileInput = modal.get('input[type="file"]');
    Object.defineProperty(fileInput.element, "files", {
      value: [new File(["dtb"], "auto-name.dtb", { type: "application/octet-stream" })],
      configurable: true,
    });

    await fileInput.trigger("change");

    expect((nameInput.element as HTMLInputElement).value).toBe("auto-name.dtb");
  });

  it("renders edit, enable/disable, and more row actions", async () => {
    const DtbView = (await import("./DtbView.vue")).default;
    const wrapper = mount(DtbView);
    await flushPromises();

    const firstRow = wrapper.find("tbody tr");
    expect(firstRow.find('button[title="编辑"]').exists()).toBe(true);
    expect(firstRow.find('button[title="禁用"]').exists()).toBe(true);
    expect(firstRow.find('button[title="更多"]').exists()).toBe(true);
  });

  it("renames, disables, and deletes an existing DTB", async () => {
    const DtbView = (await import("./DtbView.vue")).default;
    const wrapper = mount(DtbView, {
      attachTo: document.body,
    });
    await flushPromises();

    await wrapper.get('button[title="编辑"]').trigger("click");
    await flushPromises();

    const modal = wrapper.get(".modal-card");
    const renameInput = modal.get('input[placeholder="例如 rk3568-evb.dtb"]');
    await renameInput.setValue("board-v2.dtb");
    await modal.get('input[placeholder="例如 arm64 / riscv64"]').setValue("riscv64");

    await modal.get("form").trigger("submit");
    await flushPromises();

    expect(updateDtb).toHaveBeenCalledWith("board.dtb", "board-v2.dtb", null, {
      boot_architecture: "riscv64",
      compatible: "test,board",
      description: "测试开发板 DTB",
    });

    await wrapper.get('button[title="禁用"]').trigger("click");
    await flushPromises();

    expect(updateDtb).toHaveBeenCalledWith("board.dtb", null, null, {
      boot_architecture: "arm64",
      compatible: "test,board",
      description: "测试开发板 DTB",
      disabled: true,
    });

    await wrapper.get('button[title="更多"]').trigger("click");
    await flushPromises();

    const deleteItem = document.body.querySelector<HTMLButtonElement>(".action-menu .action-menu-item");
    expect(deleteItem?.textContent).toContain("删除 DTB");
    deleteItem!.click();
    await flushPromises();

    expect(deleteDtb).toHaveBeenCalledWith("board-v2.dtb");
    wrapper.unmount();
  });

  it("fills rename draft automatically after choosing a replacement file", async () => {
    const DtbView = (await import("./DtbView.vue")).default;
    const wrapper = mount(DtbView);
    await flushPromises();

    await wrapper.get('button[title="编辑"]').trigger("click");
    await flushPromises();

    const modal = wrapper.get(".modal-card");
    const renameInput = modal.get('input[placeholder="例如 rk3568-evb.dtb"]');
    const replaceFileInput = modal.get('input[type="file"]');
    Object.defineProperty(replaceFileInput.element, "files", {
      value: [new File(["dtb"], "renamed-by-file.dtb", { type: "application/octet-stream" })],
      configurable: true,
    });

    await replaceFileInput.trigger("change");

    expect((renameInput.element as HTMLInputElement).value).toBe("renamed-by-file.dtb");
  });
});

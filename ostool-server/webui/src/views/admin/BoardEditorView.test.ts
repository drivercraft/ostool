import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BoardConfig, SerialPortSummary } from "@/types/api";

const route = {
  params: {} as Record<string, string>,
};

const push = vi.fn();
const listSerialPorts = vi.fn();
const listDtbs = vi.fn();
const getTftpStatus = vi.fn();
const createDtb = vi.fn();
const getBoard = vi.fn();
const createBoard = vi.fn();
const updateBoard = vi.fn();
const deleteBoard = vi.fn();
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
  confirm: vi.fn(),
};

vi.mock("vue-router", () => ({
  useRoute: () => route,
  useRouter: () => ({ push }),
}));

vi.mock("@/api", () => ({
  api: {
    listSerialPorts,
    listDtbs,
    getTftpStatus,
    createDtb,
    getBoard,
    createBoard,
    updateBoard,
    deleteBoard,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

function makeSerialPorts(): SerialPortSummary[] {
  return [
    {
      current_device_path: "/dev/ttyUSB0",
      port_type: "usb",
      label: "[SN] abc | /dev/serial/by-path/demo / /dev/ttyUSB0 / QinHeng / USB Serial",
      primary_key_kind: "serial_number",
      primary_key_value: "abc",
      usb_path: "/dev/serial/by-path/demo",
      stable_identity: true,
      usb_vendor_id: 0x1a86,
      usb_product_id: 0x7523,
      manufacturer: "QinHeng",
      product: "USB Serial",
      serial_number: "abc",
    },
  ];
}

function makeBoard(id = "demo-board"): BoardConfig {
  return {
    id,
    board_type: "rk3568",
    tags: ["lab", "usb"],
    serial: {
      key: {
        kind: "serial_number",
        value: "abc",
      },
      baud_rate: 115200,
      resolved_device_path: "/dev/ttyUSB0",
      resolved_usb_path: "/dev/serial/by-path/demo",
    },
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
    notes: "rack-a",
    disabled: false,
  };
}

function makeStaticIpBoard(id = "static-board"): BoardConfig {
  return {
    ...makeBoard(id),
    boot: {
      kind: "uboot",
      use_tftp: true,
      dtb_name: null,
      kernel_load_addr: "0x80200000",
      fit_load_addr: "0x82200000",
      bootm_addr: "0x82200000",
      network_mode: "static_ip",
      board_ip: "192.168.10.20",
      server_ip: "192.168.10.2",
      netmask: "255.255.255.0",
      gatewayip: "192.168.10.1",
    },
  };
}

function makeRelayBoard(id = "relay-board"): BoardConfig {
  return {
    ...makeBoard(id),
    power_management: {
      kind: "zhongsheng_relay",
      key: {
        kind: "serial_number",
        value: "abc",
      },
    },
  };
}

function makeUefiHttpBoard(id = "uefi-http-board", bootArch = "x86_64"): BoardConfig {
  return {
    ...makeBoard(id),
    board_type: "Asus-nuc15-x86_64-vmx",
    power_management: {
      kind: "custom",
      power_on_cmd: "true",
      power_off_cmd: "true",
    },
    boot: {
      kind: "httpboot",
      boot_arch: bootArch,
    },
  };
}

describe("BoardEditorView", () => {
  beforeEach(() => {
    route.params = {};
    push.mockReset();
    listSerialPorts.mockReset();
    listDtbs.mockReset();
    getTftpStatus.mockReset();
    createDtb.mockReset();
    getBoard.mockReset();
    createBoard.mockReset();
    updateBoard.mockReset();
    deleteBoard.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);
    listSerialPorts.mockResolvedValue(makeSerialPorts());
    listDtbs.mockResolvedValue([]);
    getTftpStatus.mockResolvedValue({
      status: {
        provider: "builtin",
        enabled: true,
        healthy: true,
        writable: true,
        resolved_server_ip: "10.0.0.2",
        resolved_netmask: "255.255.255.0",
        root_dir: "/srv/tftp",
        bind_addr_or_address: ":69",
        service_state: null,
        last_error: null,
      },
    });
  });

  it("loads a new-board form and refreshes serial ports independently", async () => {
    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    expect(listSerialPorts).toHaveBeenCalledTimes(1);
    expect(getBoard).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("刷新串口");

    const buttons = wrapper.findAll("button");
    const refreshSerialButton = buttons.find((button) => button.text() === "刷新串口");
    expect(refreshSerialButton).toBeTruthy();

    await refreshSerialButton!.trigger("click");
    await flushPromises();

    expect(listSerialPorts).toHaveBeenCalledTimes(2);
    expect(getBoard).not.toHaveBeenCalled();
  });

  it("loads an existing board for edit mode", async () => {
    route.params = { boardId: "demo-board" };
    getBoard.mockResolvedValue(makeBoard("demo-board"));

    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    expect(getBoard).toHaveBeenCalledWith("demo-board");
    expect(
      (wrapper.get('input[placeholder="留空则自动分配 {board type}-{num}"]').element as HTMLInputElement).value,
    ).toBe("demo-board");
  });

  it("creates a board with blank id as null in the request payload", async () => {
    createBoard.mockResolvedValue(makeBoard("rk3568-1"));

    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    await wrapper.get('input[placeholder="例如 rk3568"]').setValue("rk3568");
    const textInputs = wrapper
      .findAll('input:not([type="checkbox"]):not([type="number"]):not([type="file"])');
    await textInputs[3].setValue("echo on");
    await textInputs[4].setValue("echo off");
    const saveButton = wrapper.findAll("button").find((button) => button.text() === "保存配置");
    await saveButton!.trigger("click");
    await flushPromises();

    expect(createBoard).toHaveBeenCalledWith({
      id: null,
      board_type: "rk3568",
      tags: [],
      notes: null,
      disabled: false,
      serial: null,
      power_management: {
        kind: "custom",
        power_on_cmd: "echo on",
        power_off_cmd: "echo off",
      },
      boot: {
        kind: "uboot",
        use_tftp: false,
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
    });
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已保存开发板 rk3568-1");
    expect(push).toHaveBeenCalledWith("/admin/resources/boards/rk3568-1");
  });

  it("blocks saving static IP mode without a board IP", async () => {
    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    await wrapper.get('input[placeholder="例如 rk3568"]').setValue("rk3568");
    const textInputs = wrapper
      .findAll('input:not([type="checkbox"]):not([type="number"]):not([type="file"])');
    await textInputs[3].setValue("echo on");
    await textInputs[4].setValue("echo off");
    await wrapper.get('input[aria-label="使用 TFTP 启动"]').setValue(true);
    await wrapper.get('select[aria-label="网络模式"]').setValue("static_ip");

    const saveButton = wrapper.findAll("button").find((button) => button.text() === "保存配置");
    await saveButton!.trigger("click");
    await flushPromises();

    expect(createBoard).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("静态 IP 模式必须填写开发板 IP");
  });

  it("saves static IP network fields in the board payload", async () => {
    createBoard.mockResolvedValue(makeStaticIpBoard("rk3568-1"));

    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    await wrapper.get('input[placeholder="例如 rk3568"]').setValue("rk3568");
    const textInputs = wrapper
      .findAll('input:not([type="checkbox"]):not([type="number"]):not([type="file"])');
    await textInputs[3].setValue("echo on");
    await textInputs[4].setValue("echo off");
    await wrapper.get('input[aria-label="使用 TFTP 启动"]').setValue(true);
    await wrapper.get('select[aria-label="网络模式"]').setValue("static_ip");
    await wrapper.get('input[placeholder="例如 192.168.10.20"]').setValue("192.168.10.20");
    await wrapper.get('input[placeholder="当前 serverip"]').setValue("192.168.10.2");
    await wrapper.get('input[placeholder="当前 netmask"]').setValue("255.255.255.0");
    await wrapper.get('input[placeholder="未配置"]').setValue("192.168.10.1");
    await wrapper.get('input[placeholder="例如 0x80200000"]').setValue("0x80200000");
    await wrapper.get('input[placeholder="例如 0x82200000"]').setValue("0x82200000");
    await wrapper.get('input[placeholder="默认跟随 FIT load addr"]').setValue("0x82200000");

    const saveButton = wrapper.findAll("button").find((button) => button.text() === "保存配置");
    await saveButton!.trigger("click");
    await flushPromises();

    expect(createBoard).toHaveBeenCalledWith(
      expect.objectContaining({
        boot: {
          kind: "uboot",
          use_tftp: true,
          dtb_name: null,
          kernel_load_addr: "0x80200000",
          fit_load_addr: "0x82200000",
          bootm_addr: "0x82200000",
          network_mode: "static_ip",
          board_ip: "192.168.10.20",
          server_ip: "192.168.10.2",
          netmask: "255.255.255.0",
          gatewayip: "192.168.10.1",
        },
      }),
    );
  });

  it("loads static IP network fields into the form", async () => {
    route.params = { boardId: "static-board" };
    getBoard.mockResolvedValue(makeStaticIpBoard("static-board"));

    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    expect((wrapper.get('select[aria-label="网络模式"]').element as HTMLSelectElement).value).toBe("static_ip");
    expect((wrapper.get('input[placeholder="例如 192.168.10.20"]').element as HTMLInputElement).value).toBe(
      "192.168.10.20",
    );
    expect((wrapper.get('input[placeholder="当前 serverip"]').element as HTMLInputElement).value).toBe(
      "192.168.10.2",
    );
    expect((wrapper.get('input[placeholder="当前 netmask"]').element as HTMLInputElement).value).toBe(
      "255.255.255.0",
    );
    expect((wrapper.get('input[placeholder="未配置"]').element as HTMLInputElement).value).toBe(
      "192.168.10.1",
    );
    expect((wrapper.get('input[placeholder="例如 0x80200000"]').element as HTMLInputElement).value).toBe(
      "0x80200000",
    );
    expect((wrapper.get('input[placeholder="例如 0x82200000"]').element as HTMLInputElement).value).toBe(
      "0x82200000",
    );
    expect(
      (wrapper.get('input[placeholder="默认跟随 FIT load addr"]').element as HTMLInputElement).value,
    ).toBe("0x82200000");
  });

  it("loads and saves an HTTPboot board", async () => {
    route.params = { boardId: "uefi-http-board" };
    getBoard.mockResolvedValue(makeUefiHttpBoard("uefi-http-board", "aarch64"));
    updateBoard.mockResolvedValue(makeUefiHttpBoard("uefi-http-board", "aarch64"));

    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    expect(wrapper.text()).toContain("HTTPboot");
    expect((wrapper.get('input[placeholder="例如 x86_64"]').element as HTMLInputElement).value).toBe("aarch64");
    expect(wrapper.find('input[placeholder="例如 BOOTX64.EFI"]').exists()).toBe(false);
    expect(wrapper.find('input[placeholder="例如 kernel.bin"]').exists()).toBe(false);
    expect(wrapper.find('input[placeholder="例如 1c:69:7a:dc:f3:47"]').exists()).toBe(false);

    const saveButton = wrapper.findAll("button").find((button) => button.text() === "保存配置");
    await saveButton!.trigger("click");
    await flushPromises();

    expect(updateBoard).toHaveBeenCalledWith(
      "uefi-http-board",
      expect.objectContaining({
        power_management: {
          kind: "custom",
          power_on_cmd: "true",
          power_off_cmd: "true",
        },
        boot: {
          kind: "httpboot",
          boot_arch: "aarch64",
        },
      }),
    );
  });

  it("updates a board and keeps blank id as null in the payload", async () => {
    route.params = { boardId: "demo-board" };
    getBoard.mockResolvedValue(makeBoard("demo-board"));
    updateBoard.mockResolvedValue(makeBoard("demo-board"));

    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    await wrapper.get('input[placeholder="留空则自动分配 {board type}-{num}"]').setValue("");
    const saveButton = wrapper.findAll("button").find((button) => button.text() === "保存配置");
    await saveButton!.trigger("click");
    await flushPromises();

    expect(updateBoard).toHaveBeenCalledWith("demo-board", expect.objectContaining({ id: null }));
    expect(push).toHaveBeenCalledWith("/admin/resources/boards/demo-board");
  });

  it("saves relay power management using a stable serial key", async () => {
    route.params = { boardId: "relay-board" };
    getBoard.mockResolvedValue(makeRelayBoard("relay-board"));
    updateBoard.mockResolvedValue(makeRelayBoard("relay-board"));

    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    const saveButton = wrapper.findAll("button").find((button) => button.text() === "保存配置");
    await saveButton!.trigger("click");
    await flushPromises();

    expect(updateBoard).toHaveBeenCalledWith(
      "relay-board",
      expect.objectContaining({
        power_management: {
          kind: "zhongsheng_relay",
          key: {
            kind: "serial_number",
            value: "abc",
          },
        },
      }),
    );
  });

  it("blocks saving when required power management fields are empty", async () => {
    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    await wrapper.get('input[placeholder="例如 rk3568"]').setValue("rk3568");
    const saveButton = wrapper.findAll("button").find((button) => button.text() === "保存配置");
    await saveButton!.trigger("click");
    await flushPromises();

    expect(createBoard).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("Custom 电源管理必须填写开机命令");
  });

  it("fills upload DTB name automatically after choosing a file", async () => {
    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    const openButton = wrapper.findAll("button").find((button) => button.text() === "新增 DTB");
    await openButton!.trigger("click");
    await flushPromises();

    const modal = wrapper.get(".modal-card");
    const uploadNameInput = modal.get('input[placeholder="例如 board.dtb"]');
    const uploadFileInput = modal.get('input[type="file"]');
    Object.defineProperty(uploadFileInput.element, "files", {
      value: [new File(["dtb"], "picked-board.dtb", { type: "application/octet-stream" })],
      configurable: true,
    });

    await uploadFileInput.trigger("change");

    expect((uploadNameInput.element as HTMLInputElement).value).toBe("picked-board.dtb");
  });

  it("uploads DTB with selected filename even if the name field stays blank", async () => {
    createDtb.mockResolvedValue({
      name: "picked-board.dtb",
      size: 3,
      updated_at: "2026-04-01T00:00:00Z",
      relative_tftp_path_template: "boot/dtb/picked-board.dtb",
    });

    const BoardEditorView = (await import("./BoardEditorView.vue")).default;
    const wrapper = mount(BoardEditorView);
    await flushPromises();

    const openButton = wrapper.findAll("button").find((button) => button.text() === "新增 DTB");
    await openButton!.trigger("click");
    await flushPromises();

    const modal = wrapper.get(".modal-card");
    const uploadFileInput = modal.get('input[type="file"]');
    Object.defineProperty(uploadFileInput.element, "files", {
      value: [new File(["dtb"], "picked-board.dtb", { type: "application/octet-stream" })],
      configurable: true,
    });
    await uploadFileInput.trigger("change");
    await modal.get('input[placeholder="例如 board.dtb"]').setValue("");

    const uploadButton = modal.findAll("button").find((button) => button.text() === "上传并选中");
    await uploadButton!.trigger("click");
    await flushPromises();

    expect(createDtb).toHaveBeenCalledWith("picked-board.dtb", expect.any(File));
  });
});

import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const getServerConfig = vi.fn();
const updateServerConfig = vi.fn();
const listNetworkInterfaces = vi.fn();
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
};

vi.mock("@/api/client", () => ({
  api: {
    getServerConfig,
    updateServerConfig,
    listNetworkInterfaces,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

function makeConfig() {
  return {
    readonly: {
      listen_addr: "0.0.0.0:2999",
      data_dir: "/var/lib/ostool-server",
      board_dir: "/var/lib/ostool-server/boards",
      dtb_dir: "/var/lib/ostool-server/dtbs",
      dtb_upload_max_mib: 10,
    },
    editable: {
      network: {
        interface: "eth0",
      },
      upload_limits: {
        session_file_max_mib: 64,
      },
    },
  };
}

describe("ServerView", () => {
  beforeEach(() => {
    getServerConfig.mockReset();
    updateServerConfig.mockReset();
    listNetworkInterfaces.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();

    getServerConfig.mockResolvedValue(makeConfig());
    listNetworkInterfaces.mockResolvedValue([
      {
        name: "eth0",
        label: "eth0",
        ipv4_addresses: ["192.168.1.10"],
        netmask: "255.255.255.0",
        loopback: false,
      },
    ]);
    updateServerConfig.mockImplementation(async (payload) => ({
      ...makeConfig(),
      editable: payload,
    }));
  });

  it("loads config, renders fixed DTB limit, and saves upload limits", async () => {
    const ServerView = (await import("./ServerView.vue")).default;
    const wrapper = mount(ServerView);
    await flushPromises();

    expect(getServerConfig).toHaveBeenCalledTimes(1);
    expect(listNetworkInterfaces).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain("DTB 上传上限");
    expect(wrapper.text()).toContain("10 MiB");

    const numberInput = wrapper.get('input[type="number"]');
    await numberInput.setValue("32");

    const saveButton = wrapper.findAll("button").find((button) => button.text() === "保存配置");
    await saveButton!.trigger("click");
    await flushPromises();

    expect(updateServerConfig).toHaveBeenCalledWith({
      network: {
        interface: "eth0",
      },
      upload_limits: {
        session_file_max_mib: 32,
      },
    });
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已保存 Server 安全配置");
  });
});

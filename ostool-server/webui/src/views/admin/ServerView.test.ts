import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const getServerConfig = vi.fn();
const updateServerConfig = vi.fn();
const listNetworkInterfaces = vi.fn();
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
  confirm: vi.fn(),
};

vi.mock("@/api", () => ({
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
    site: {
      site_name: "ostool-server",
      site_subtitle: "开发板租赁平台",
      logo_url: null,
      favicon_url: null,
      announcement: null,
      maintenance_mode: false,
      self_service_enabled: true,
      default_lease_minutes: 120,
      max_lease_minutes: 480,
      support_email: null,
      support_url: null,
      updated_at: "2026-01-01T00:00:00Z",
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
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);

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
      editable: {
        network: payload.network,
        upload_limits: payload.upload_limits,
      },
      site: {
        ...payload.site,
        updated_at: "2026-01-01T00:00:00Z",
      },
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

    const numberInputs = wrapper.findAll('input[type="number"]');
    const numberInput = numberInputs[numberInputs.length - 1];
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
      site: {
        site_name: "ostool-server",
        site_subtitle: "开发板租赁平台",
        logo_url: null,
        favicon_url: null,
        announcement: null,
        maintenance_mode: false,
        self_service_enabled: true,
        default_lease_minutes: 120,
        max_lease_minutes: 480,
        support_email: null,
        support_url: null,
        updated_at: "2026-01-01T00:00:00Z",
      },
    });
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已保存系统设置");
  });
});

import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listBoardTypes = vi.fn();
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
  confirm: vi.fn(),
};

vi.mock("@/api", () => ({
  api: { listBoardTypes },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

const RouterLinkStub = {
  name: "RouterLink",
  props: ["to"],
  template: "<a><slot /></a>",
};

async function seedUser() {
  const { useAuthStore } = await import("@/stores/auth");
  const store = useAuthStore();
  store.user = {
    id: "user-demo",
    username: "demo",
    display_name: "Demo",
    nickname: null,
    avatar_url: null,
    email: "demo@ostool.local",
    phone: null,
    department: null,
    title: null,
    last_login_at: null,
    roles: [],
    permissions: [],
  };
  store.loaded = true;
}

describe("ResourcesView", () => {
  beforeEach(() => {
    listBoardTypes.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);

    listBoardTypes.mockResolvedValue([
      {
        board_type: "rk3568",
        tags: ["lab"],
        total: 4,
        available: 2,
      },
      {
        board_type: "stm32mp1",
        tags: [],
        total: 2,
        available: 0,
      },
    ]);
  });

  it("renders board types and computes totals", async () => {
    const ResourcesView = (await import("./ResourcesView.vue")).default;
    const wrapper = mount(ResourcesView, {
      global: { stubs: { RouterLink: RouterLinkStub } },
    });
    await flushPromises();

    expect(listBoardTypes).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain("rk3568");
    expect(wrapper.text()).toContain("stm32mp1");
    expect(wrapper.text()).toContain("型号");
    expect(wrapper.text()).toContain("在管总数");
    // 新版统计卡按 label/value 结构展示，按顺序取每张卡的数值
    const statValues = wrapper.findAll(".resource-stat-value").map((el) => el.text());
    expect(statValues).toEqual(["2 款", "2 块", "4 块", "6 块"]);
  });

  it("filters out unavailable boards when toggling availability filter", async () => {
    const ResourcesView = (await import("./ResourcesView.vue")).default;
    const wrapper = mount(ResourcesView, {
      global: { stubs: { RouterLink: RouterLinkStub } },
    });
    await flushPromises();

    expect(wrapper.text()).toContain("stm32mp1");

    const select = wrapper.findAll("select")[0];
    await select.setValue("available");
    await flushPromises();

    expect(wrapper.text()).toContain("rk3568");
    expect(wrapper.text()).not.toContain("stm32mp1");
  });

  it("links available resources to the standalone lease creation page", async () => {
    await seedUser();
    const ResourcesView = (await import("./ResourcesView.vue")).default;
    const wrapper = mount(ResourcesView, {
      global: { stubs: { RouterLink: RouterLinkStub } },
    });
    await flushPromises();

    const leaseLink = wrapper
      .findAllComponents(RouterLinkStub)
      .find((link) => link.text().includes("申请租赁"));

    expect(leaseLink?.props("to")).toEqual({
      name: "user-lease-new",
      query: { board_type: "rk3568" },
    });
  });
});

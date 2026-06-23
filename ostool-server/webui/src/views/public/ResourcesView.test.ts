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
  template: "<a><slot /></a>",
};

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
    expect(wrapper.findAll(".stats-num").map((el) => el.text())).toEqual([
      "2",
      "2",
      "4",
      "6",
    ]);
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
});

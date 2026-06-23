import { mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";

vi.mock("vue-router", () => ({
  RouterLink: {
    template: "<a><slot /></a>",
  },
}));

describe("HomeView", () => {
  it("renders hero copy and capability sections without fetching board resources", async () => {
    const HomeView = (await import("./HomeView.vue")).default;
    const wrapper = mount(HomeView);

    expect(wrapper.text()).toContain("面向团队的开发板租赁与远程调试平台");
    expect(wrapper.text()).toContain("把硬件实验室变成可调度的共享资源");
    expect(wrapper.text()).toContain("从镜像上传到远程启动的完整闭环");
    // 不再展示具体板型资源
    expect(wrapper.text()).not.toContain("rk3568");
  });

  it("renders getting started steps and CTA actions", async () => {
    const HomeView = (await import("./HomeView.vue")).default;
    const wrapper = mount(HomeView);

    expect(wrapper.text()).toContain("四步即可上手一块开发板");
    expect(wrapper.text()).toContain("浏览资源");
    expect(wrapper.text()).toContain("立即登录");
  });
});

import { mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";

vi.mock("vue-router", () => ({
  RouterLink: {
    props: ["to"],
    template: "<a :href='to'><slot /></a>",
  },
}));

describe("legal pages", () => {
  it("renders platform-specific terms", async () => {
    const TermsView = (await import("./TermsView.vue")).default;
    const wrapper = mount(TermsView);

    expect(wrapper.text()).toContain("用户协议");
    expect(wrapper.text()).toContain("开发板预约、租赁、远程调试");
    expect(wrapper.text()).toContain("远程串口");
    expect(wrapper.text()).toContain("TFTP / HTTP Boot");
  });

  it("renders platform-specific privacy policy", async () => {
    const PrivacyView = (await import("./PrivacyView.vue")).default;
    const wrapper = mount(PrivacyView);

    expect(wrapper.text()).toContain("隐私政策");
    expect(wrapper.text()).toContain("租赁 ID");
    expect(wrapper.text()).toContain("来源 IP");
    expect(wrapper.text()).toContain("会话临时文件");
  });
});

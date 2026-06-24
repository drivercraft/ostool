import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const route = { query: {} };
const routerPush = vi.fn();
const getCaptcha = vi.fn();
const authStore = {
  isAdmin: false,
  login: vi.fn(),
};
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
};

vi.mock("vue-router", () => ({
  RouterLink: {
    props: ["to"],
    template: "<a :href='to'><slot /></a>",
  },
  useRoute: () => route,
  useRouter: () => ({
    push: routerPush,
  }),
}));

vi.mock("@/stores/auth", () => ({
  useAuthStore: () => authStore,
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("@/api", () => ({
  api: {
    getCaptcha,
  },
}));

describe("auth legal links", () => {
  beforeEach(() => {
    routerPush.mockReset();
    getCaptcha.mockReset();
    getCaptcha.mockResolvedValue({
      token: "captcha-token",
      image_svg: "<svg></svg>",
      expires_in_seconds: 300,
    });
    authStore.login.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
  });

  it("shows terms and privacy links on login", async () => {
    const LoginView = (await import("./LoginView.vue")).default;
    const wrapper = mount(LoginView);

    expect(wrapper.find('a[href="/terms"]').text()).toBe("用户协议");
    expect(wrapper.find('a[href="/privacy"]').text()).toBe("隐私政策");
    expect(wrapper.text()).toContain("验证码");
    expect(getCaptcha).toHaveBeenCalledTimes(1);
  });

  it("shows terms and privacy links on register agreement", async () => {
    const RegisterView = (await import("./RegisterView.vue")).default;
    const wrapper = mount(RegisterView);

    expect(wrapper.find('a[href="/terms"]').text()).toBe("用户协议");
    expect(wrapper.find('a[href="/privacy"]').text()).toBe("隐私政策");
    expect(wrapper.text()).toContain("验证码");
    expect(getCaptcha).toHaveBeenCalledTimes(1);
  });
});

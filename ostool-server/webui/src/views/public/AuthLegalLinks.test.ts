import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const route = { query: {} };
const routerPush = vi.fn();
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

describe("auth legal links", () => {
  beforeEach(() => {
    routerPush.mockReset();
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
  });

  it("shows terms and privacy links on register agreement", async () => {
    const RegisterView = (await import("./RegisterView.vue")).default;
    const wrapper = mount(RegisterView);

    expect(wrapper.find('a[href="/terms"]').text()).toBe("用户协议");
    expect(wrapper.find('a[href="/privacy"]').text()).toBe("隐私政策");
  });
});

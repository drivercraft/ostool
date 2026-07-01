import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const routerPush = vi.fn();
const getCaptcha = vi.fn();
const getRegistrationPolicy = vi.fn();
const register = vi.fn();
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
  useRouter: () => ({ push: routerPush }),
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("@/api", () => ({
  api: {
    auth: {
      getCaptcha,
      getRegistrationPolicy,
      register,
    },
  },
}));

async function mountRegister() {
  const RegisterView = (await import("./RegisterView.vue")).default;
  const wrapper = mount(RegisterView);
  await flushPromises();
  return wrapper;
}

function fillForm(wrapper: ReturnType<typeof mount>) {
  wrapper.find('input[autocomplete="username"]').setValue("bob");
  wrapper.find('input[placeholder="用于页面展示，例如：张三"]').setValue("Bob");
  wrapper.find('input[type="email"]').setValue("bob@example.com");
  wrapper.find('input[autocomplete="new-password"]').setValue("supersecret");
  wrapper.findAll('input[autocomplete="new-password"]')[1].setValue("supersecret");
  wrapper.find('input[autocomplete="off"]').setValue("ABCDEF");
  wrapper.find('input[type="checkbox"]').setValue(true);
}

describe("RegisterView", () => {
  beforeEach(() => {
    routerPush.mockReset();
    getCaptcha.mockReset();
    getRegistrationPolicy.mockReset();
    register.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    getCaptcha.mockResolvedValue({
      token: "captcha-token",
      image_svg: "<svg></svg>",
      expires_in_seconds: 300,
    });
  });

  it("hides the form and shows a notice when registration is closed", async () => {
    getRegistrationPolicy.mockResolvedValue({
      mode: "closed",
      self_service_enabled: false,
    });
    const wrapper = await mountRegister();

    expect(getCaptcha).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("已关闭自助注册");
    expect(wrapper.find("form").exists()).toBe(false);
  });

  it("submits a registration and routes to login on auto outcome", async () => {
    getRegistrationPolicy.mockResolvedValue({
      mode: "auto",
      self_service_enabled: true,
    });
    register.mockResolvedValue({
      outcome: "active",
      username: "bob",
      display_name: "Bob",
    });
    const wrapper = await mountRegister();

    fillForm(wrapper);
    await wrapper.find("form").trigger("submit.prevent");
    await flushPromises();

    expect(register).toHaveBeenCalledTimes(1);
    expect(uiStore.setSuccess).toHaveBeenCalled();
    expect(routerPush).toHaveBeenCalledWith(
      expect.objectContaining({ name: "login" }),
    );
  });

  it("shows a pending message when approval is required", async () => {
    getRegistrationPolicy.mockResolvedValue({
      mode: "approval",
      self_service_enabled: true,
    });
    register.mockResolvedValue({
      outcome: "pending",
      username: "bob",
      display_name: "Bob",
    });
    const wrapper = await mountRegister();

    fillForm(wrapper);
    await wrapper.find("form").trigger("submit.prevent");
    await flushPromises();

    expect(register).toHaveBeenCalledTimes(1);
    expect(uiStore.setSuccess).toHaveBeenCalledWith(
      expect.stringContaining("等待管理员审核"),
    );
  });
});

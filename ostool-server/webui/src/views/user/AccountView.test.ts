import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const updateUserPassword = vi.fn();
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
};
const routerReplace = vi.fn();

vi.mock("@/api", () => ({
  api: {
    user: {
      updateUserPassword,
    },
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("vue-router", () => ({
  useRouter: () => ({ replace: routerReplace }),
}));

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
    department: "研发部",
    title: "工程师",
    last_login_at: null,
    roles: [{ id: "role-user", name: "user", display_name: "普通用户", description: "", system: true, disabled: false, user_count: 0, permissions: [], created_at: "", updated_at: "" }],
    permissions: [],
  };
  store.loaded = true;
}

describe("AccountView", () => {
  beforeEach(() => {
    updateUserPassword.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    routerReplace.mockReset();
    updateUserPassword.mockResolvedValue(undefined);
  });

  it("renders the current user's account information", async () => {
    await seedUser();
    const AccountView = (await import("./AccountView.vue")).default;
    const wrapper = mount(AccountView);
    await flushPromises();

    expect(wrapper.text()).toContain("基本资料");
    expect(wrapper.text()).toContain("账号安全");
    expect(wrapper.text()).toContain("demo");
    expect(wrapper.text()).toContain("Demo");
    expect(wrapper.text()).toContain("demo@ostool.local");
    expect(wrapper.text()).toContain("修改密码");
    expect(wrapper.findAll('input[type="password"]')).toHaveLength(3);
  });

  it("validates repeated password input before changing password", async () => {
    await seedUser();
    const AccountView = (await import("./AccountView.vue")).default;
    const wrapper = mount(AccountView);
    await flushPromises();

    const inputs = wrapper.findAll('input[type="password"]');
    await inputs[0].setValue("old-password-1");
    await inputs[1].setValue("new-password-1");
    await inputs[2].setValue("new-password-2");

    expect(wrapper.text()).toContain("两次输入的新密码不一致");
  });

  it("updates the current user's password", async () => {
    await seedUser();
    const AccountView = (await import("./AccountView.vue")).default;
    const wrapper = mount(AccountView);
    await flushPromises();

    const inputs = wrapper.findAll('input[type="password"]');
    await inputs[0].setValue("old-password-1");
    await inputs[1].setValue("new-password-1");
    await inputs[2].setValue("new-password-1");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(updateUserPassword).toHaveBeenCalledWith({
      current_password: "old-password-1",
      new_password: "new-password-1",
      confirm_new_password: "new-password-1",
    });
    expect(uiStore.setSuccess).toHaveBeenCalledWith("密码已修改");
  });
});

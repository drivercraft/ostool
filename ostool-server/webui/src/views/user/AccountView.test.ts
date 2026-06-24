import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const uiStore = {
  clearMessages: vi.fn(),
};
const routerReplace = vi.fn();

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
    roles: [{ id: "role-user", name: "user", display_name: "普通用户", description: "", system: true, user_count: 0, permissions: [], created_at: "", updated_at: "" }],
    permissions: [],
  };
  store.loaded = true;
}

describe("AccountView", () => {
  beforeEach(() => {
    uiStore.clearMessages.mockReset();
    routerReplace.mockReset();
  });

  it("renders the current user's account information", async () => {
    await seedUser();
    const AccountView = (await import("./AccountView.vue")).default;
    const wrapper = mount(AccountView);
    await flushPromises();

    expect(wrapper.text()).toContain("账户信息");
    expect(wrapper.text()).toContain("demo");
    expect(wrapper.text()).toContain("Demo");
    expect(wrapper.text()).toContain("demo@ostool.local");
    expect(wrapper.text()).toContain("修改密码");
  });
});

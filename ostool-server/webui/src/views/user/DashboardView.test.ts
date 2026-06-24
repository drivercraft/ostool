import { beforeEach, describe, expect, it, vi } from "vitest";

const listBoardTypes = vi.fn();
const listUserLeases = vi.fn();
const createLease = vi.fn();
const deleteLease = vi.fn();
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
  confirm: vi.fn(),
};
const routerPush = vi.fn();
const routerReplace = vi.fn();

vi.mock("@/api", () => ({
  api: {
    listBoardTypes,
    listUserLeases,
    createLease,
    deleteLease,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("vue-router", () => ({
  useRouter: () => ({ push: routerPush, replace: routerReplace }),
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
    department: null,
    title: null,
    last_login_at: null,
    roles: [{ id: "role-user", name: "user", display_name: "普通用户", description: "", system: true, user_count: 0, permissions: [], created_at: "", updated_at: "" }],
    permissions: [],
  };
  store.loaded = true;
}

describe("DashboardView", () => {
  beforeEach(() => {
    listBoardTypes.mockReset();
    listUserLeases.mockReset();
    createLease.mockReset();
    deleteLease.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    uiStore.confirm.mockReset();
    uiStore.confirm.mockResolvedValue(true);
    routerPush.mockReset();
    routerReplace.mockReset();

    listBoardTypes.mockResolvedValue([
      {
        board_type: "rk3568",
        tags: ["lab"],
        total: 2,
        available: 1,
      },
    ]);
    listUserLeases.mockResolvedValue({ leases: [] });
  });

  it("redirects anonymous users to /login", async () => {
    const { flushPromises, mount } = await import("@vue/test-utils");
    const DashboardView = (await import("./DashboardView.vue")).default;
    mount(DashboardView);
    await flushPromises();
    expect(routerReplace).toHaveBeenCalledWith("/login");
  });

  it("loads board types and creates a session for an authenticated user", async () => {
    await seedUser();
    const { flushPromises, mount } = await import("@vue/test-utils");
    const DashboardView = (await import("./DashboardView.vue")).default;
    createLease.mockResolvedValue({
      lease: {
        id: "lease-1",
        user_id: "user-demo",
        session_id: "session-2",
        board_id: "rk3568-1",
        board_type: "rk3568",
        required_tags: [],
        state: "active",
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        starts_at: "2026-01-01T00:00:00Z",
        expires_at: new Date(Date.now() + 1800_000).toISOString(),
        released_at: null,
        failure_message: null,
      },
      session: null,
    });

    const wrapper = mount(DashboardView);
    await flushPromises();

    expect(listBoardTypes).toHaveBeenCalled();
    expect(wrapper.text()).toContain("Demo");

    const select = wrapper.get("select");
    await select.setValue("rk3568");

    const button = wrapper
      .findAll("button")
      .find((btn) => btn.text() === "申请会话");
    await button!.trigger("click");
    await flushPromises();

    expect(createLease).toHaveBeenCalledWith({
      board_type: "rk3568",
      required_tags: [],
    });
    expect(uiStore.setSuccess).toHaveBeenCalled();
  });
});

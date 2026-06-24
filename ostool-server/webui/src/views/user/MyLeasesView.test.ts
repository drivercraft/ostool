import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listUserLeases = vi.fn();
const deleteLease = vi.fn();
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
  confirm: vi.fn(),
};
const routerReplace = vi.fn();

vi.mock("@/api", () => ({
  api: {
    listUserLeases,
    deleteLease,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("vue-router", () => ({
  useRouter: () => ({ replace: routerReplace }),
  RouterLink: {
    props: ["to"],
    template: "<a><slot /></a>",
  },
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
    roles: [],
    permissions: [],
  };
  store.loaded = true;
}

function makeLease() {
  return {
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
    session: {
      id: "session-2",
      board_id: "rk3568-1",
      client_name: "demo",
      source_ip: "127.0.0.1",
      created_at: "2026-01-01T00:00:00Z",
      expires_at: new Date(Date.now() + 1800_000).toISOString(),
      state: "active",
    },
  };
}

describe("MyLeasesView", () => {
  beforeEach(() => {
    [listUserLeases, deleteLease, uiStore.clearMessages, uiStore.setError, uiStore.setSuccess, uiStore.confirm, routerReplace]
      .forEach((fn) => fn.mockReset());
    uiStore.confirm.mockResolvedValue(true);
    listUserLeases.mockResolvedValue({ leases: [makeLease()] });
  });

  it("renders leases and current sessions, then releases a lease", async () => {
    await seedUser();
    const MyLeasesView = (await import("./MyLeasesView.vue")).default;
    const wrapper = mount(MyLeasesView);
    await flushPromises();

    expect(wrapper.text()).toContain("我的租赁");
    expect(wrapper.text()).toContain("我的预约日历");
    expect(wrapper.text()).toContain("租赁情况");
    expect(wrapper.text()).toContain("租约会话");
    expect(wrapper.text()).toContain("rk3568-1");
    expect(wrapper.find(".lease-calendar-month").exists()).toBe(true);

    await wrapper.findAll(".lease-calendar-tabs button")[0].trigger("click");
    await flushPromises();
    expect(wrapper.find(".lease-calendar-hour").exists()).toBe(true);
    expect(wrapper.findAll(".lease-calendar-cell")).toHaveLength(24);

    await wrapper.findAll(".lease-calendar-tabs button")[1].trigger("click");
    await flushPromises();
    expect(wrapper.find(".lease-calendar-day").exists()).toBe(true);
    expect(wrapper.findAll(".lease-calendar-cell")).toHaveLength(63);

    const button = wrapper
      .findAll("button")
      .find((btn) => btn.text() === "释放租赁");
    await button!.trigger("click");
    await flushPromises();

    expect(deleteLease).toHaveBeenCalledWith("lease-1");
    expect(uiStore.setSuccess).toHaveBeenCalled();
  });
});

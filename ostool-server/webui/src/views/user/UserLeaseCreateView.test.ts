import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listBoardTypes = vi.fn();
const listUserLeases = vi.fn();
const createLease = vi.fn();
const routerPush = vi.fn();
let routeMock = {
  query: { board_type: "rk3568" } as Record<string, string>,
};
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
};

vi.mock("@/api", () => ({
  api: {
    listBoardTypes,
    listUserLeases,
    createLease,
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("vue-router", async () => {
  const actual = await vi.importActual<typeof import("vue-router")>("vue-router");
  return {
    ...actual,
    useRoute: () => routeMock,
    useRouter: () => ({ push: routerPush }),
    RouterLink: {
      props: ["to"],
      template: "<a><slot /></a>",
    },
  };
});

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

describe("UserLeaseCreateView", () => {
  beforeEach(() => {
    routeMock = {
      query: { board_type: "rk3568" },
    };
    [listBoardTypes, listUserLeases, createLease, routerPush, uiStore.clearMessages, uiStore.setError, uiStore.setSuccess]
      .forEach((fn) => fn.mockReset());
    listBoardTypes.mockResolvedValue([
      { board_type: "rk3568", tags: ["lab"], total: 2, available: 1 },
      { board_type: "stm32mp1", tags: [], total: 1, available: 0 },
    ]);
    listUserLeases.mockResolvedValue({
      leases: [
        {
          lease: {
            id: "lease-1",
            user_id: "user-demo",
            session_id: "session-1",
            board_id: "rk3568-1",
            board_type: "rk3568",
            required_tags: [],
            state: "active",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-01T00:00:00Z",
            starts_at: "2026-01-01T00:00:00Z",
            expires_at: "2026-01-01T02:00:00Z",
            released_at: null,
            failure_message: null,
          },
          session: null,
        },
      ],
    });
    createLease.mockResolvedValue({
      lease: {
        id: "lease-created",
        user_id: "user-demo",
        session_id: "session-created",
        board_id: "rk3568-2",
        board_type: "rk3568",
        required_tags: ["lab"],
        state: "active",
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        starts_at: "2026-01-01T03:00:00Z",
        expires_at: "2026-01-01T04:00:00Z",
        released_at: null,
        failure_message: null,
      },
      session: null,
    });
  });

  it("renders a standalone lease creation page and creates a user lease", async () => {
    await seedUser();
    const UserLeaseCreateView = (await import("./UserLeaseCreateView.vue")).default;
    const wrapper = mount(UserLeaseCreateView);
    await flushPromises();

    expect(wrapper.text()).toContain("申请租赁");
    expect(wrapper.find(".lease-calendar-month").exists()).toBe(true);
    expect((wrapper.find("select").element as HTMLSelectElement).value).toBe("rk3568");

    await wrapper.find('input[placeholder="多个标签用英文逗号分隔，例如 lab, usb"]').setValue("lab");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(createLease).toHaveBeenCalledWith({
      board_type: "rk3568",
      required_tags: ["lab"],
    });
    expect(routerPush).toHaveBeenCalledWith("/dashboard#leases");
  });
});

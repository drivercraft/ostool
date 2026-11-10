import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { BrowserRouter, Routes, Route } from "react-router-dom";
import MemberWorkspace from "./MemberWorkspace";
import RentalRequest from "./RentalRequest";
const mocks = vi.hoisted(() => ({
  user: {
    id: "member",
    email: "member@test.local",
    display_name: "Original",
    is_admin: false,
    permissions: [] as string[],
  },
  updateProfile: vi.fn(),
  applyRental: vi.fn(),
  claimBoard: vi.fn(),
  withdrawRentalRequest: vi.fn(),
  resources: {} as Record<string, unknown>,
  errors: {},
  listeners: new Set<() => void>(),
}));
vi.mock("@/apps/web/session", () => ({
  useSiteSession: () => ({ user: mocks.user, checking: false, error: "" }),
}));
vi.mock("@/api/client", () => ({
  api: {
    updateProfile: mocks.updateProfile,
    applyRental: mocks.applyRental,
    claimBoard: mocks.claimBoard,
    withdrawRentalRequest: mocks.withdrawRentalRequest,
  },
  clearAuthToken: vi.fn(),
}));
vi.mock("@/api/events", async () => {
  const { useSyncExternalStore } = await import("react");
  const subscribe = (fn: () => void) => {
    mocks.listeners.add(fn);
    return () => {
      mocks.listeners.delete(fn);
    };
  };
  return {
    useAccountResource: (key: string) =>
      useSyncExternalStore(subscribe, () => mocks.resources[key]),
    useDeviceDocument: () => undefined,
    site: { setDeviceDocument: vi.fn() },
    useAccountConnection: () => "connected",
    useAccountErrors: () => mocks.errors,
    account: {
      subscribe,
      get: (key: string) => mocks.resources[key],
      getErrors: () => mocks.errors,
      setListingFilters: vi.fn(),
      setEditorFilters: (query: unknown) => {
        if (
          !query ||
          JSON.stringify(query) ===
            JSON.stringify(mocks.resources.availability_query)
        )
          return;
        mocks.resources.availability_query = query;
        queueMicrotask(() => mocks.listeners.forEach((fn) => fn()));
      },
    },
  };
});
beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  for (const key of Object.keys(mocks.resources)) delete mocks.resources[key];
  localStorage.clear();
  vi.clearAllMocks();
  mocks.updateProfile.mockResolvedValue(undefined);
  mocks.applyRental.mockResolvedValue(undefined);
  mocks.user.display_name = "Original";
  mocks.user.permissions = [];
  mocks.claimBoard.mockResolvedValue({
    id: "claim",
    board_id: "shared",
    ends_at: "2026-11-09T23:00:00Z",
  });
  window.history.replaceState({}, "", "/account/profile");
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
const App = () => (
  <BrowserRouter>
    <Routes>
      <Route path="/rental-request" element={<RentalRequest />} />
      <Route path="/account/*" element={<MemberWorkspace />} />
    </Routes>
  </BrowserRouter>
);
it("preserves a profile draft across server pushes and submits only editable fields", async () => {
  const user = userEvent.setup();
  const app = render(<App />);
  const name = screen.getByLabelText("显示名称");
  await user.clear(name);
  await user.type(name, "My unsaved name");
  mocks.user.display_name = "A server update";
  app.rerender(<App />);
  expect((name as HTMLInputElement).value).toBe("My unsaved name");
  await user.click(screen.getByRole("button", { name: "保存资料" }));
  expect(mocks.updateProfile).toHaveBeenCalledWith({
    email: "member@test.local",
    display_name: "My unsaved name",
    current_password: undefined,
  });
});
it("requires the current password when the login email changes", async () => {
  const user = userEvent.setup();
  render(<App />);
  await user.clear(screen.getByLabelText("邮箱"));
  await user.type(screen.getByLabelText("邮箱"), "new@test.local");
  expect(
    (screen.getByLabelText("当前密码（修改邮箱时必填）") as HTMLInputElement)
      .required,
  ).toBe(true);
});

it("uses the rental editor layout and preserves the application draft on availability pushes", async () => {
  mocks.resources.catalog = [{ id: "board-1", board_type: "test", tags: [] }];
  mocks.resources.availability = [];
  mocks.resources.my_requests = [];
  window.history.replaceState({}, "", "/rental-request?board=board-1");
  const app = render(<App />),
    user = userEvent.setup();
  expect(screen.getByRole("heading", { name: "租赁申请" })).toBeTruthy();
  expect(screen.queryByRole("navigation", { name: "个人中心导航" })).toBeNull();
  expect(document.querySelector(".member-workspace")).toBeNull();
  const notes = screen.getByLabelText("申请说明");
  await user.type(notes, "Keep this application draft");
  const from = (screen.getByLabelText("开始时间") as HTMLInputElement).value;
  const to = (screen.getByLabelText("结束时间") as HTMLInputElement).value;
  expect(screen.getByRole("region", { name: "租赁日历" })).toBeTruthy();
  expect(
    screen.getByRole("button", { name: "提交申请" }).closest(".panel-footer"),
  ).toBeTruthy();
  expect(notes.closest(".panel-toolbar")).toBeTruthy();
  mocks.resources.availability = [
    {
      id: "occupied",
      board_id: "board-1",
      starts_at: new Date(from).toISOString(),
      ends_at: new Date(to).toISOString(),
      status: "scheduled",
    },
  ];
  app.rerender(<App />);
  expect((notes as HTMLInputElement).value).toBe("Keep this application draft");
  expect(
    (screen.getByRole("button", { name: "提交申请" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  mocks.resources.availability = [];
  app.rerender(<App />);
  await user.click(screen.getByRole("button", { name: "提交申请" }));
  expect(mocks.applyRental).toHaveBeenCalledWith({
    board_id: "board-1",
    starts_at: new Date(from).toISOString(),
    ends_at: new Date(to).toISOString(),
    notes: "Keep this application draft",
  });
  await screen.findByRole("heading", { name: "租赁申请" });
  expect(window.location.pathname).toBe("/account/requests");
});

it("claims an eligible board for the current user and reacts to availability changes", async () => {
  mocks.user.permissions = ["board.use", "board.instant"];
  mocks.resources.catalog = [
    { id: "shared", board_type: "arm", tags: [], instant_available: true },
    { id: "reserved", board_type: "arm", tags: [], instant_available: false },
  ];
  window.history.replaceState({}, "", "/account/allocations/new");
  render(<App />);
  const user = userEvent.setup();
  expect(screen.queryByRole("option", { name: /reserved/ })).toBeNull();
  await user.selectOptions(screen.getByLabelText("开发板"), "shared");
  await user.click(screen.getByRole("button", { name: "确认领取" }));
  expect(mocks.claimBoard).toHaveBeenCalledWith("shared");
});
it("rejects the direct claim page when the role lacks instant allocation", () => {
  window.history.replaceState({}, "", "/account/allocations/new");
  render(<App />);
  expect(
    screen.getByText("当前角色没有即时分配权限，请通过租赁申请获取设备。"),
  ).toBeTruthy();
  expect(screen.queryByRole("button", { name: "确认领取" })).toBeNull();
});

it.each([false, true])(
  "the workspace links to the shared application page with instant access=%s",
  async (instant) => {
    mocks.user.permissions = instant ? ["board.instant"] : [];
    mocks.resources.catalog = [{ id: "board-1", board_type: "test", tags: [] }];
    mocks.resources.availability = [];
    window.history.replaceState({}, "", "/account");
    render(<App />);
    const user = userEvent.setup();
    const application = screen.getByRole("link", {
      name: "申请租赁",
    });
    expect(application.getAttribute("href")).toBe("/rental-request");
    if (instant)
      expect(
        screen
          .getByRole("link", { name: "即时领取开发板" })
          .getAttribute("href"),
      ).toBe("/account/allocations/new");
    await user.click(application);
    await screen.findByRole("button", { name: "提交申请" });
    expect(window.location.pathname).toBe("/rental-request");
    expect(
      screen.queryByRole("navigation", { name: "个人中心导航" }),
    ).toBeNull();
  },
);

it("keeps rentals and applications on separate pages with their own active navigation", async () => {
  window.history.replaceState({}, "", "/account/requests");
  render(<App />);
  await screen.findByRole("heading", { name: "租赁申请" });
  const navigation = within(
    screen.getByRole("navigation", { name: "个人中心导航" }),
  );
  const requests = navigation.getByRole("link", { name: "租赁申请" });
  const rentals = navigation.getByRole("link", { name: "我的租赁" });
  expect(requests.getAttribute("aria-current")).toBe("page");
  expect(rentals.getAttribute("aria-current")).toBeNull();
  expect(screen.getByLabelText("申请状态")).toBeTruthy();
  expect(screen.queryByLabelText("搜索我的租赁")).toBeNull();
  expect(screen.queryByRole("tab")).toBeNull();
  expect(
    screen.getByRole("link", { name: "申请租赁" }).getAttribute("href"),
  ).toBe("/rental-request");
  const user = userEvent.setup();
  await user.click(rentals);
  expect(window.location.pathname).toBe("/account/rentals");
  expect(rentals.getAttribute("aria-current")).toBe("page");
  expect(requests.getAttribute("aria-current")).toBeNull();
  expect(screen.getByRole("heading", { name: "我的租赁" })).toBeTruthy();
  expect(screen.getByLabelText("搜索我的租赁")).toBeTruthy();
  expect(screen.queryByLabelText("申请状态")).toBeNull();
  await user.click(requests);
  expect(window.location.pathname).toBe("/account/requests");
  expect(screen.getByRole("heading", { name: "租赁申请" })).toBeTruthy();
});
it("keeps pending application withdrawal on the applications page", async () => {
  mocks.resources.listing = {
    query: { view: "rental_requests", page: 1, page_size: 20 },
    page: 1,
    page_size: 20,
    total: 1,
    items: [
      {
        id: "pending-request",
        board_id: "board-1",
        starts_at: "2026-11-10T10:00:00Z",
        ends_at: "2026-11-10T12:00:00Z",
        status: "pending",
        notes: "Need a board",
        decision_note: null,
      },
    ],
  };
  mocks.withdrawRentalRequest.mockResolvedValue(undefined);
  window.history.replaceState({}, "", "/account/requests");
  render(<App />);
  expect(within(screen.getByRole("table")).getByText("待审核")).toBeTruthy();
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "撤回申请" }));
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", { name: "确认" }),
  );
  expect(mocks.withdrawRentalRequest).toHaveBeenCalledWith("pending-request");
});

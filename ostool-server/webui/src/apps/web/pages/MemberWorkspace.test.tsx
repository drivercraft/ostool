import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { BrowserRouter, Routes, Route } from "react-router-dom";
import MemberWorkspace from "./MemberWorkspace";
const mocks = vi.hoisted(() => ({
  user: {
    id: "member",
    email: "member@test.local",
    display_name: "Original",
    is_admin: false,
  },
  updateProfile: vi.fn(),
  applyRental: vi.fn(),
  resources: {} as Record<string, unknown>,
}));
vi.mock("@/apps/web/session", () => ({
  useSiteSession: () => ({ user: mocks.user, checking: false, error: "" }),
}));
vi.mock("@/api/client", () => ({
  api: { updateProfile: mocks.updateProfile, applyRental: mocks.applyRental },
  clearAuthToken: vi.fn(),
}));
vi.mock("@/api/events", () => ({
  useAccountResource: (key: string) => mocks.resources[key],
  useAccountConnection: () => "connected",
  useAccountErrors: () => ({}),
}));
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
  window.history.replaceState({}, "", "/account/profile");
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
const App = () => (
  <BrowserRouter>
    <Routes>
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
  window.history.replaceState({}, "", "/account/requests/new?board=board-1");
  const app = render(<App />),
    user = userEvent.setup();
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
  await screen.findByRole("link", { name: "新增申请" });
  expect(window.location.pathname).toBe("/account/requests");
});

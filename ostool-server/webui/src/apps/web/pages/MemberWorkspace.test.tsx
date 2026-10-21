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
  resources: {} as Record<string, unknown>,
}));
vi.mock("@/apps/web/session", () => ({
  useSiteSession: () => ({ user: mocks.user, checking: false, error: "" }),
}));
vi.mock("@/api/client", () => ({
  api: { updateProfile: mocks.updateProfile },
  clearAuthToken: vi.fn(),
}));
vi.mock("@/api/events", () => ({
  useAccountResource: (key: string) => mocks.resources[key],
  useAccountConnection: () => "connected",
  useAccountErrors: () => ({}),
}));
beforeEach(() => {
  vi.clearAllMocks();
  mocks.updateProfile.mockResolvedValue(undefined);
  mocks.user.display_name = "Original";
  window.history.replaceState({}, "", "/account/profile");
});
afterEach(cleanup);
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

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import PublicApp from "./PublicApp";

const mocks = vi.hoisted(() => ({
  config: vi.fn(),
  register: vi.fn(),
  login: vi.fn(),
  me: vi.fn(),
}));
vi.mock("./api/client", () => ({
  api: {
    authConfig: mocks.config,
    register: mocks.register,
    login: mocks.login,
    me: mocks.me,
  },
  saveAuthToken: vi.fn(),
}));
beforeEach(() => {
  vi.clearAllMocks();
  mocks.config.mockResolvedValue({
    enabled: true,
    allow_registration: true,
    database_available: true,
  });
  mocks.register.mockResolvedValue({ access_token: "member-token" });
  mocks.me.mockResolvedValue({
    email: "member@test.local",
    display_name: "Member",
    is_admin: false,
  });
  window.history.replaceState({}, "", "/login");
});
afterEach(cleanup);

describe("registration", () => {
  it("switches a mounted login page to registration through the header", async () => {
    const user = userEvent.setup();
    render(<PublicApp />);
    await user.click(screen.getAllByRole("link", { name: "免费注册" })[0]);
    expect(
      await screen.findByRole("heading", { name: "创建你的账号" }),
    ).toBeTruthy();
    expect(screen.getByLabelText("确认密码")).toBeTruthy();
  });
  it("validates confirmation and sends a real registration request then opens member account", async () => {
    window.history.replaceState({}, "", "/register");
    const user = userEvent.setup();
    render(<PublicApp />);
    await user.type(screen.getByLabelText("邮箱"), "member@test.local");
    await user.type(screen.getByLabelText("显示名称"), "Member");
    await user.type(screen.getByLabelText("密码"), "member-password-123");
    await user.type(screen.getByLabelText("确认密码"), "different-password");
    await user.click(screen.getByRole("button", { name: /创建账号/ }));
    expect(screen.getByRole("alert").textContent).toContain(
      "两次输入的密码不一致",
    );
    expect(mocks.register).not.toHaveBeenCalled();
    await user.clear(screen.getByLabelText("确认密码"));
    await user.type(screen.getByLabelText("确认密码"), "member-password-123");
    await user.click(screen.getByRole("button", { name: /创建账号/ }));
    await waitFor(() => expect(window.location.pathname).toBe("/account"));
    expect(mocks.register).toHaveBeenCalledWith(
      "member@test.local",
      "member-password-123",
      "Member",
    );
  });
  it("shows registration closed without sending a request", async () => {
    mocks.config.mockResolvedValue({ allow_registration: false });
    window.history.replaceState({}, "", "/register");
    render(<PublicApp />);
    await screen.findByText("当前站点未开放注册，请联系管理员。");
    expect(
      (screen.getByRole("button", { name: /创建账号/ }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    expect(mocks.register).not.toHaveBeenCalled();
  });
});

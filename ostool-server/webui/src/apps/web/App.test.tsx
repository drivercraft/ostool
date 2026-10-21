import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import PublicApp from "@/apps/web/App";

const mocks = vi.hoisted(() => ({
  config: vi.fn(),
  register: vi.fn(),
  login: vi.fn(),
  me: vi.fn(),
}));
vi.mock("@/api/events", () => ({
  account: {
    connect: vi.fn(),
    disconnect: vi.fn(),
    reset: vi.fn(),
    get: vi.fn(),
  },
  useAccountResource: () => undefined,
  useAccountConnection: () => "connected",
  useAccountErrors: () => ({}),
}));
vi.mock("@/api/client", () => ({
  api: {
    authConfig: mocks.config,
    register: mocks.register,
    login: mocks.login,
    me: mocks.me,
  },
  saveAuthToken: (token: { access_token: string }) => {
    localStorage.setItem("ostool.access_token", token.access_token);
    window.dispatchEvent(new Event("ostool:auth"));
  },
  clearAuthToken: () => {
    localStorage.clear();
    window.dispatchEvent(new Event("ostool:auth"));
  },
}));
beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    value: vi.fn().mockImplementation(() => ({
      matches: false,
      addListener: vi.fn(),
      removeListener: vi.fn(),
    })),
  });
  mocks.config.mockResolvedValue({
    edition: "public",
    enabled: true,
    allow_registration: true,
    database_available: true,
  });
  mocks.register.mockResolvedValue({ access_token: "member-token" });
  mocks.login.mockResolvedValue({ access_token: "member-token" });
  mocks.me.mockResolvedValue({
    id: "member",
    email: "member@test.local",
    display_name: "Member",
    is_admin: false,
  });
  window.history.replaceState({}, "", "/login");
});
afterEach(cleanup);

describe("registration", () => {
  it("keeps the account route pending while a newly saved login identity loads", async () => {
    let finish!: (value: unknown) => void;
    mocks.me.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    render(<PublicApp />);
    const user = userEvent.setup();
    await user.type(await screen.findByLabelText("邮箱"), "member@test.local");
    await user.type(screen.getByLabelText("密码"), "member-password-123");
    await user.click(screen.getByRole("button", { name: "登录" }));
    await waitFor(() => expect(window.location.pathname).toBe("/account"));
    expect(screen.queryByRole("heading", { name: "当前账号" })).toBeNull();
    finish({
      id: "member",
      email: "member@test.local",
      display_name: "Member",
      is_admin: false,
    });
    await screen.findByRole("heading", { name: "当前账号" });
    expect(window.location.pathname).toBe("/account");
  });
  it("keeps administrators in the public workspace after a normal website login", async () => {
    mocks.me.mockResolvedValue({
      id: "admin",
      email: "admin@test.local",
      display_name: "Admin",
      is_admin: true,
    });
    const user = userEvent.setup();
    render(<PublicApp />);
    await user.type(await screen.findByLabelText("邮箱"), "admin@test.local");
    await user.type(screen.getByLabelText("密码"), "admin-password-123");
    await user.click(screen.getByRole("button", { name: "登录" }));
    await screen.findByRole("heading", { name: "当前账号" });
    expect(window.location.pathname).toBe("/account");
    expect(
      screen.getAllByRole("button", { name: "用户菜单" }).length,
    ).toBeGreaterThan(0);
    expect(screen.queryByRole("link", { name: "登录" })).toBeNull();
  });
  it("hides every account entry in local edition, even with PostgreSQL and a stale token", async () => {
    mocks.config.mockResolvedValue({
      edition: "local",
      enabled: false,
      allow_registration: false,
      database_available: true,
    });
    localStorage.setItem("ostool.access_token", "stale-token");
    render(<PublicApp />);
    await screen.findByRole("heading", { name: /让每一块开发板/ });
    expect(window.location.pathname).toBe("/");
    expect(screen.queryByRole("link", { name: "免费注册" })).toBeNull();
    expect(screen.queryByRole("link", { name: "登录" })).toBeNull();
    expect(mocks.me).not.toHaveBeenCalled();
    expect(mocks.login).not.toHaveBeenCalled();
    localStorage.clear();
  });
  it("switches a mounted login page to registration through the header", async () => {
    const user = userEvent.setup();
    render(<PublicApp />);
    await user.click(
      (await screen.findAllByRole("link", { name: "免费注册" }))[0],
    );
    expect(
      await screen.findByRole("heading", { name: "创建你的账号" }),
    ).toBeTruthy();
    expect(screen.getByLabelText("确认密码")).toBeTruthy();
  });
  it("validates confirmation and sends a real registration request then opens member account", async () => {
    window.history.replaceState({}, "", "/register");
    const user = userEvent.setup();
    render(<PublicApp />);
    await user.type(await screen.findByLabelText("邮箱"), "member@test.local");
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
    mocks.config.mockResolvedValue({
      edition: "public",
      allow_registration: false,
    });
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

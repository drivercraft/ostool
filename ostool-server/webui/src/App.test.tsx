import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import App from "./App";
const mocks = vi.hoisted(() => ({
  config: vi.fn(),
  me: vi.fn(),
  connect: vi.fn(),
  disconnect: vi.fn(),
}));
vi.mock("@/api/client", () => ({
  api: { authConfig: mocks.config, me: mocks.me },
}));
vi.mock("@/api/events", () => ({
  admin: { connect: mocks.connect, disconnect: mocks.disconnect },
  useConnection: () => "connected",
  useErrors: () => ({}),
}));
vi.mock("./pages/Overview", () => ({ default: () => <div>本地设备总览</div> }));
beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  window.history.replaceState({}, "", "/admin/overview");
});
afterEach(cleanup);
describe("edition navigation and authentication", () => {
  it("local edition opens without login and hides user and rental modules", async () => {
    mocks.config.mockResolvedValue({
      edition: "local",
      enabled: false,
      database_available: true,
    });
    render(<App />);
    await screen.findByText("本地版 · 免登录");
    expect(screen.queryByText("用户管理")).toBeNull();
    expect(screen.queryByText("租赁管理")).toBeNull();
    expect(screen.queryByText("我的账号")).toBeNull();
    expect(mocks.me).not.toHaveBeenCalled();
    await waitFor(() => expect(mocks.connect).toHaveBeenCalledTimes(1));
  });
  it("public member cannot mount the management console or event stream", async () => {
    mocks.config.mockResolvedValue({
      edition: "public",
      enabled: true,
      database_available: true,
    });
    localStorage.setItem("ostool.access_token", "member");
    mocks.me.mockResolvedValue({ is_admin: false });
    render(<App />);
    await screen.findByText("你的账号没有管理后台权限，请联系管理员分配角色。");
    expect(mocks.connect).not.toHaveBeenCalled();
  });
  it("public administrator sees rental and user management", async () => {
    mocks.config.mockResolvedValue({
      edition: "public",
      enabled: true,
      database_available: true,
    });
    localStorage.setItem("ostool.access_token", "admin");
    mocks.me.mockResolvedValue({ is_admin: true });
    render(<App />);
    await screen.findByRole("link", { name: "租赁列表" });
    expect(screen.getByRole("link", { name: "用户列表" })).toBeTruthy();
    expect(screen.getByRole("link", { name: "用户角色" })).toBeTruthy();
  });
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  act,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import App from "@/apps/admin/App";
import userEvent from "@testing-library/user-event";
const mocks = vi.hoisted(() => ({
  config: vi.fn(),
  me: vi.fn(),
  connect: vi.fn(),
  disconnect: vi.fn(),
  connection: vi.fn(),
}));
vi.mock("@/api/client", () => ({
  api: { authConfig: mocks.config, me: mocks.me },
}));
vi.mock("@/api/events", () => ({
  site: { connect: vi.fn(), disconnect: vi.fn() },
  useSiteResource: () => undefined,
  admin: {
    setTopics: vi.fn(),
    setBoardEditor: vi.fn(),
    connect: mocks.connect,
    disconnect: mocks.disconnect,
  },
  useConnection: () => mocks.connection(),
  useErrors: () => ({}),
  useResource: () => undefined,
}));
vi.mock("@/apps/admin/pages/Overview", () => ({
  default: () => <div>本地设备总览</div>,
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
  vi.stubGlobal(
    "matchMedia",
    vi.fn().mockImplementation((query) => ({
      matches: false,
      media: query,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  );
  vi.clearAllMocks();
  mocks.connection.mockReturnValue("connected");
  localStorage.clear();
  window.history.replaceState({}, "", "/qcl/overview");
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  document.documentElement.classList.remove("dark", "light");
});
describe("edition navigation and authentication", () => {
  it("keeps an open theme menu mounted when delayed authentication completes", async () => {
    mocks.config.mockResolvedValue({ edition: "public", enabled: true, database_available: true });
    let finishAuth!: (user: { is_admin: boolean }) => void;
    mocks.me.mockImplementation(() => new Promise((resolve) => { finishAuth = resolve; }));
    render(<App />);
    const user = userEvent.setup();
    const trigger = await screen.findByRole("button", { name: "切换主题" });
    expect(document.querySelector(".admin-workspace")?.getAttribute("aria-busy")).toBe("true");
    expect(mocks.connect).not.toHaveBeenCalled();
    await user.click(trigger);
    expect(await screen.findByRole("menuitemradio", { name: "跟随系统" })).toBeTruthy();
    await act(async () => { finishAuth({ is_admin: true }); });
    await screen.findByText("本地设备总览");
    expect(screen.getByRole("button", { name: "切换主题" })).toBe(trigger);
    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    await user.click(screen.getByRole("menuitemradio", { name: "跟随系统" }));
    expect(localStorage.getItem("ostool.admin.theme")).toBe("system");
    expect(mocks.connect).toHaveBeenCalledTimes(1);
  });
  it("shows the shared 404 inside the console and returns to an allowed page", async () => {
    mocks.config.mockResolvedValue({ edition: "local", enabled: false, database_available: true });
    window.history.replaceState({}, "", "/qcl/missing-page");
    render(<App />);
    await screen.findByRole("heading", { name: "页面不存在" });
    expect(document.querySelector("#admin-sidebar")).toBeTruthy();
    expect(window.location.pathname).toBe("/qcl/missing-page");
    await userEvent.click(screen.getByRole("link", { name: "返回管理后台" }));
    await screen.findByText("本地设备总览");
    expect(window.location.pathname).toBe("/qcl/overview");
  });
  it("local edition opens without login and hides account and allocation management", async () => {
    mocks.config.mockResolvedValue({edition:"local",enabled:false,database_available:true});
    mocks.me.mockResolvedValue({id:"local-admin",username:"admin",email:"admin@test.local",is_admin:true});
    render(<App />);
    await screen.findByText("本地设备总览");
    expect(mocks.me).not.toHaveBeenCalled();
    expect(screen.queryByText("用户管理")).toBeNull();
    expect(screen.queryByText("租赁管理")).toBeNull();
    await waitFor(() => expect(mocks.connect).toHaveBeenCalledTimes(1));
  });
  it("does not show a global disconnection banner above page content", async () => {
    mocks.config.mockResolvedValue({
      edition: "local",
      enabled: true,
      database_available: true,
    });
    mocks.me.mockResolvedValue({id:"local-admin",username:"admin",email:"admin@test.local",is_admin:true});
    mocks.connection.mockReturnValue("reconnecting");
    render(<App />);
    await screen.findByText("本地设备总览");
    expect(document.querySelector(".admin-notices")?.textContent).toBe("");
    expect(
      screen.queryByText(
        "连接暂时中断，当前内容和输入已保留，恢复后自动同步。",
      ),
    ).toBeNull();
  });
  it("public member cannot mount the management console or event stream", async () => {
    mocks.config.mockResolvedValue({
      edition: "public",
      enabled: true,
      database_available: true,
    });
    localStorage.setItem("ostool.admin.access_token", "member");
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
    localStorage.setItem("ostool.admin.access_token", "admin");
    mocks.me.mockResolvedValue({ is_admin: true });
    render(<App />);
    await screen.findByRole("link", { name: "租赁列表" });
    expect(screen.getByRole("link", { name: "用户列表" })).toBeTruthy();
    expect(screen.getByRole("link", { name: "用户角色" })).toBeTruthy();
  });
});

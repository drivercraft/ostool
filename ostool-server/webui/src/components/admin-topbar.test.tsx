import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { ThemeProvider } from "next-themes";
import { AdminTopbar } from "./admin-topbar";

const mocks = vi.hoisted(() => ({
  edition: "local",
  errors: {} as Record<string, string>,
  resources: {} as Record<string, unknown>,
  logout: vi.fn(),
}));
vi.mock("@/edition", () => ({
  useEdition: () => ({ edition: mocks.edition }),
}));
vi.mock("@/api/client", () => ({ api: { logout: mocks.logout } }));
vi.mock("@/api/events", () => ({
  useErrors: () => mocks.errors,
  useConnection: () => "connected",
  useResource: (key: string) => mocks.resources[key],
}));
function Wrapper() {
  return (
    <MemoryRouter initialEntries={["/users/roles"]}>
      <ThemeProvider
        attribute="class"
        storageKey="ostool.admin.theme"
        defaultTheme="system"
      >
        <AdminTopbar
          menuOpen={false}
          onMenuToggle={() => {}}
          user={
            mocks.edition === "public"
              ? {
                  id: "admin",
                  email: "admin@example.com",
                  display_name: "Ada",
                  is_admin: true,
                }
              : undefined
          }
        />
      </ThemeProvider>
    </MemoryRouter>
  );
}
beforeEach(() => {
  mocks.edition = "local";
  mocks.errors = {};
  mocks.resources = {};
  vi.clearAllMocks();
  localStorage.clear();
  vi.stubGlobal(
    "matchMedia",
    vi.fn().mockImplementation((query) => ({
      matches: false,
      media: query,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
  // Radix menus use pointer-capture APIs that jsdom does not implement.
  HTMLElement.prototype.hasPointerCapture = vi.fn(() => false);
  HTMLElement.prototype.setPointerCapture = vi.fn();
  HTMLElement.prototype.releasePointerCapture = vi.fn();
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  document.documentElement.classList.remove("dark", "light");
});
describe("application titlebar", () => {
  it("shows an unread badge from live errors, clears it on read and detects changed messages", async () => {
    const user = userEvent.setup();
    mocks.errors = { tftp: "TFTP 服务启动失败" };
    const view = render(<Wrapper />);
    expect(screen.getByRole("heading", { name: "用户角色" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "消息，1 条未读" })).toBeTruthy();
    expect(screen.getByTestId("notification-badge").textContent).toBe("1");
    await user.click(screen.getByRole("button", { name: "消息，1 条未读" }));
    expect(await screen.findByText("TFTP 服务启动失败")).toBeTruthy();
    expect(screen.queryByTestId("notification-badge")).toBeNull();
    await user.keyboard("{Escape}");
    mocks.errors = { tftp: "TFTP 连接异常" };
    view.rerender(<Wrapper />);
    expect(screen.getByRole("button", { name: "消息，1 条未读" })).toBeTruthy();
    mocks.errors = {};
    view.rerender(<Wrapper />);
    expect(screen.getByRole("button", { name: "消息，0 条未读" })).toBeTruthy();
  });
  it("persists theme selection and supports Escape dismissal", async () => {
    const user = userEvent.setup();
    render(<Wrapper />);
    await user.click(screen.getByRole("button", { name: "切换主题" }));
    await user.click(
      await screen.findByRole("menuitemradio", { name: "黑暗" }),
    );
    await waitFor(() =>
      expect(document.documentElement.classList.contains("dark")).toBe(true),
    );
    expect(localStorage.getItem("ostool.admin.theme")).toBe("dark");
    await user.click(screen.getByRole("button", { name: "切换主题" }));
    await user.click(
      await screen.findByRole("menuitemradio", { name: "跟随系统" }),
    );
    await waitFor(() =>
      expect(document.documentElement.classList.contains("dark")).toBe(false),
    );
    expect(localStorage.getItem("ostool.admin.theme")).toBe("system");
    await user.click(screen.getByRole("button", { name: "切换主题" }));
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  });
  it("local avatar menu exposes workspace actions without account or logout", async () => {
    const user = userEvent.setup();
    render(<Wrapper />);
    await user.click(screen.getByRole("button", { name: "账号菜单" }));
    expect(await screen.findByText("本地版 · 免登录")).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "系统设置" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "访问网站" })).toBeTruthy();
    expect(screen.queryByRole("menuitem", { name: "我的账号" })).toBeNull();
    expect(screen.queryByRole("menuitem", { name: "退出登录" })).toBeNull();
  });
  it("public avatar uses the authenticated identity and account links", async () => {
    mocks.edition = "public";
    const user = userEvent.setup();
    render(<Wrapper />);
    expect(screen.getByRole("button", { name: "账号菜单" }).textContent).toBe(
      "A",
    );
    await user.click(screen.getByRole("button", { name: "账号菜单" }));
    expect(await screen.findByText("admin@example.com")).toBeTruthy();
    expect(
      screen.getByRole("menuitem", { name: "我的账号" }).getAttribute("href"),
    ).toBe("/account");
    expect(screen.getByRole("menuitem", { name: "退出登录" })).toBeTruthy();
  });
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import PublicApp from "@/apps/web/App";

const emptyErrors = {};
const mocks = vi.hoisted(() => ({
  config: vi.fn(),
  register: vi.fn(),
  login: vi.fn(),
  me: vi.fn(),
  captcha: vi.fn(),
  authorizeDevice: vi.fn(),
  devices: [] as {
    id: string;
    board_type: string;
    tags: string[];
    disabled: boolean;
  }[],
}));
vi.mock("@/api/events", () => ({
  site: {
    connect: vi.fn(),
    disconnect: vi.fn(),
    subscribe: () => () => {},
    getErrors: () => emptyErrors,
    getConnection: () => "connected",
  },
  usePublicDevices: () => mocks.devices,
  useSiteResource: () => undefined,
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
    captcha: mocks.captcha,
    authorizeDevice: mocks.authorizeDevice,
  },
  notifyAuthChanged: () => {
    localStorage.setItem("ostool.website.session-state", "signed-in");
    window.dispatchEvent(new Event("ostool:auth"));
  },
  clearBrowserSession: () => {
    localStorage.removeItem("ostool.website.session-state");
    window.dispatchEvent(new Event("ostool:auth"));
  },
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
  vi.clearAllMocks();
  mocks.devices = [];
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
  mocks.register.mockResolvedValue({ authenticated: true });
  mocks.login.mockResolvedValue({ authenticated: true });
  mocks.authorizeDevice.mockResolvedValue(undefined);
  mocks.captcha.mockImplementation((action: string) =>
    Promise.resolve({
      id: "00000000-0000-0000-0000-000000000001",
      action,
      provider: "image",
      image_data_url: "data:image/png;base64,iVBORw0KGgo=",
      expires_in: 120,
    }),
  );
  mocks.me.mockImplementation(() =>
    localStorage.getItem("ostool.website.session-state")
      ? Promise.resolve({
          id: "member",
          email: "member@test.local",
          display_name: "Member",
          is_admin: false,
        })
      : Promise.reject(
          Object.assign(new Error("not logged in"), { status: 401 }),
        ),
  );
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
    await user.type(await screen.findByLabelText("图形验证码"), "AB29Q");
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
    await user.type(await screen.findByLabelText("图形验证码"), "AB29Q");
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
    if (!(screen.getByLabelText("图形验证码") as HTMLInputElement).value)
      await user.type(screen.getByLabelText("图形验证码"), "AB29Q");
    await user.click(
      screen.getByRole("checkbox", {
        name: "我已阅读并同意用户协议和隐私政策",
      }),
    );
    await user.click(screen.getByRole("button", { name: /创建账号/ }));
    expect(screen.getByRole("alert").textContent).toContain(
      "两次输入的密码不一致",
    );
    expect(mocks.register).not.toHaveBeenCalled();
    await user.clear(screen.getByLabelText("确认密码"));
    await user.type(screen.getByLabelText("确认密码"), "member-password-123");
    if (!(screen.getByLabelText("图形验证码") as HTMLInputElement).value)
      await user.type(screen.getByLabelText("图形验证码"), "AB29Q");
    await user.click(screen.getByRole("button", { name: /创建账号/ }));
    await waitFor(() => expect(window.location.pathname).toBe("/account"));
    expect(mocks.register).toHaveBeenCalledWith(
      "member@test.local",
      "member-password-123",
      "Member",
      {
        captcha_id: "00000000-0000-0000-0000-000000000001",
        captcha_code: "AB29Q",
      },
      undefined,
      { accepted: true, terms_version: 1, privacy_version: 1 },
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

describe("public device directory", () => {
  it("shows all devices as cards without authentication and preserves the selected board through login", async () => {
    mocks.devices = [
      {
        id: "rk board/1",
        board_type: "rk3568",
        tags: ["arm64"],
        disabled: false,
      },
      { id: "offline-board", board_type: "qemu", tags: [], disabled: true },
    ];
    window.history.replaceState({}, "", "/devices");
    render(<PublicApp />);
    await screen.findByRole("heading", { name: "rk board/1" });
    expect(screen.getByRole("heading", { name: "offline-board" })).toBeTruthy();
    expect(document.querySelectorAll(".device-directory-card")).toHaveLength(2);
    expect(screen.queryByRole("table")).toBeNull();
    expect(
      (screen.getByRole("button", { name: "暂不可申请" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    const link = screen.getByRole("link", { name: "登录后申请" });
    expect(
      new URL(
        link.getAttribute("href")!,
        window.location.origin,
      ).searchParams.get("redirect"),
    ).toBe("/account/requests/new?board=rk%20board%2F1");
    const user = userEvent.setup();
    await user.type(screen.getByLabelText("搜索设备"), "arm64");
    expect(screen.queryByRole("heading", { name: "offline-board" })).toBeNull();
  });
  it("keeps the same catalog in local edition without rental or login actions", async () => {
    mocks.config.mockResolvedValue({
      edition: "local",
      enabled: false,
      allow_registration: false,
    });
    mocks.devices = [
      {
        id: "local-board",
        board_type: "local-test",
        tags: [],
        disabled: false,
      },
    ];
    window.history.replaceState({}, "", "/devices");
    render(<PublicApp />);
    await screen.findByRole("heading", { name: "local-board" });
    expect(screen.getByRole("link", { name: "查看设备" })).toBeTruthy();
    expect(
      screen.queryByRole("link", { name: /申请租赁|登录后申请/ }),
    ).toBeNull();
    expect(mocks.me).not.toHaveBeenCalled();
  });
});

describe("CLI device authorization", () => {
  it("requires explicit confirmation before logging in and approving a terminal code", async () => {
    window.history.replaceState({}, "", "/login?user_code=ABCD-EFGH");
    const user = userEvent.setup();
    render(<PublicApp />);
    await user.type(await screen.findByLabelText("邮箱"), "member@test.local");
    await user.type(screen.getByLabelText("密码"), "member-password-123");
    await user.type(await screen.findByLabelText("图形验证码"), "AB29Q");
    const submit = screen.getByRole("button", { name: "登录并批准" });
    expect((submit as HTMLButtonElement).disabled).toBe(true);
    await user.click(submit);
    expect(mocks.login).not.toHaveBeenCalled();
    expect(mocks.authorizeDevice).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole("checkbox", { name: "确认由我发起并核对终端验证码" }),
    );
    await user.click(submit);
    await screen.findByText("设备登录已批准，可以返回 ostool CLI。");
    expect(mocks.authorizeDevice).toHaveBeenCalledWith("ABCD-EFGH");
  });
});

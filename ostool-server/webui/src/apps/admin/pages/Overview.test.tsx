import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import Overview from "./Overview";
import type { DashboardInfo } from "@/types/api";

const mocks = vi.hoisted(() => ({
  connection: "connected",
  errors: {} as Record<string, string>,
  restart: vi.fn(),
  data: undefined as DashboardInfo | undefined,
}));
vi.mock("@/api/events", () => ({
  admin: { restart: mocks.restart },
  useConnection: () => mocks.connection,
  useErrors: () => mocks.errors,
  useResource: () => mocks.data,
}));
vi.mock("@/edition", () => ({ useEdition: () => ({ edition: "local" }) }));

beforeEach(() => {
  mocks.connection = "connected";
  mocks.errors = {};
  mocks.data = {
    runtime: {
      started_at: "2026-10-22T00:00:00Z",
      uptime_secs: 3600,
      version: "0.9.1",
      platform: "linux",
      arch: "x86_64",
      pid: 1,
      logical_cpus: 4,
      sample: {
        ready: true,
        sampled_at: "2026-10-22T01:00:00Z",
        database_ok: true,
        redis_ok: true,
      },
      history: [],
      tasks: [],
      http: {
        requests: 20,
        errors: 0,
        inflight: 0,
        mean_latency_ms: 1,
        sse_connections: 1,
      },
    },
    operations: {
      boards: 2,
      board_states: { idle: 2 },
      sessions: 0,
      devices: 0,
      online_devices: 0,
      conflicting_devices: 0,
      dtbs: 0,
      public: false,
    },
    audit: { healthy: true, write_failures: 0 },
    audit_retention_days: null,
    power_actions: [],
    ota_jobs: [],
  };
});
afterEach(cleanup);
const App = () => (
  <MemoryRouter>
    <Overview />
  </MemoryRouter>
);
const summary = () => screen.getByRole("region", { name: "运行状态汇总" });

describe("runtime connection health", () => {
  it("replaces cached healthy status on disconnect and restores it on synchronization", () => {
    const app = render(<App />);
    expect(within(summary()).getByRole("status").textContent).toBe(
      "服务运行正常",
    );
    mocks.connection = "reconnecting";
    app.rerender(<App />);
    expect(within(summary()).getByRole("status").textContent).toBe(
      "连接中断，正在重连",
    );
    expect(screen.queryByText("服务运行正常")).toBeNull();
    expect(screen.getAllByText("待重新确认")).toHaveLength(3);
    expect(screen.getByText("连接中断，自动重连中")).toBeTruthy();
    // Runtime metrics remain visible while the transport is unavailable.
    expect(screen.getByText("2 台开发板 · 2 台可用 · 0 个会话")).toBeTruthy();
    mocks.connection = "connected";
    app.rerender(<App />);
    expect(within(summary()).getByRole("status").textContent).toBe(
      "服务运行正常",
    );
    expect(screen.queryByText("待重新确认")).toBeNull();
  });

  it("does not report healthy while connecting, synchronization fails, or a dependency fails", () => {
    mocks.connection = "connecting";
    const app = render(<App />);
    expect(within(summary()).getByRole("status").textContent).toBe(
      "正在连接服务器",
    );
    mocks.connection = "connected";
    mocks.errors = { dashboard: "database unavailable" };
    app.rerender(<App />);
    expect(within(summary()).getByRole("status").textContent).toBe(
      "运行状态同步异常",
    );
    expect(screen.queryByText("服务运行正常")).toBeNull();
    mocks.errors = {};
    mocks.data!.runtime.sample.redis_ok = false;
    app.rerender(<App />);
    expect(within(summary()).getByRole("status").textContent).toBe(
      "服务依赖状态待确认",
    );
    expect(screen.getByText("连接异常")).toBeTruthy();
  });
});

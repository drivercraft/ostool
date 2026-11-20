import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { SessionStatus } from "./session-status";
import type { AdminSession } from "@/types/api";

afterEach(cleanup);
const session: AdminSession = {
  id: "session",
  board_id: "board",
  state: "active",
  created_at: "2026-11-20T00:00:00Z",
  expires_at: "2026-11-20T01:00:00Z",
  client_name: "ostool",
  owner_username: "user",
  owner_user_id: "user",
  rental_id: null,
  rental_starts_at: null,
  rental_ends_at: null,
  serial_connected: false,
  serial_runtime: {
    phase: "waiting_device",
    port: null,
    parameters: null,
    error: null,
    warning: null,
  },
};

it("shows one primary status and no default device-wait badge for a disconnected session", () => {
  const view = render(<SessionStatus session={session} automaticSerial />);
  expect(screen.getByText("占用中")).toBeTruthy();
  expect(screen.queryByText("等待设备")).toBeNull();
  expect(view.container.querySelectorAll('[data-slot="badge"]')).toHaveLength(
    1,
  );
});

it("prioritizes release failure and hides all connection and binding status", () => {
  render(
    <SessionStatus
      session={{ ...session, state: "releasing", serial_connected: true }}
      automaticSerial
      runtime={{
        lease_state: "error",
        active_session_id: session.id,
        last_release_error: "设备释放失败，请检查服务器日志",
      }}
    />,
  );
  expect(screen.getByText("释放失败")).toBeTruthy();
  expect(screen.getByText("设备释放失败，请检查服务器日志")).toBeTruthy();
  expect(screen.queryByText(/终端.*连接/)).toBeNull();
  expect(screen.queryByText("等待设备")).toBeNull();
});

it("does not attach a different session's release error to the current session", () => {
  render(
    <SessionStatus
      session={{ ...session, serial_connected: true }}
      automaticSerial={false}
      runtime={{
        lease_state: "error",
        active_session_id: "old-session",
        last_release_error: "过期错误",
      }}
    />,
  );
  expect(screen.getByText("使用中")).toBeTruthy();
  expect(screen.queryByText("过期错误")).toBeNull();
});

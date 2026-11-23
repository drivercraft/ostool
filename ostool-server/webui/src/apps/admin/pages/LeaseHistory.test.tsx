import "@testing-library/jest-dom/vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { afterEach, expect, it, vi } from "vitest";
import { admin } from "@/api/events";
import { api } from "@/api/client";
import type { LeaseRecord } from "@/types/api";
import LeaseHistory from "./LeaseHistory";
vi.mock("@/edition", () => ({ useEdition: () => ({ enabled: false }) }));
vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    disconnect() {}
  },
);
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  sessionStorage.clear();
});
const record: LeaseRecord = {
  session_id: "ended-session",
  board_id: "board one",
  board_type: "test",
  user_id: null,
  username: null,
  rental_id: null,
  client_name: "client",
  created_at: "2026-11-01T00:00:00Z",
  last_heartbeat_at: "2026-11-01T01:00:00Z",
  expires_at: "2026-11-01T02:00:00Z",
  stop_requested_at: "2026-11-01T01:00:00Z",
  ended_at: "2026-11-01T01:00:00Z",
  stop_reason: "admin_released",
  end_reason: "admin_released",
  release_failures: 0,
  last_release_error: null,
};
function push(items: LeaseRecord[]) {
  admin.apply({
    epoch: crypto.randomUUID(),
    revision: 1,
    kind: "snapshot",
    data: {
      session_history: {
        active: true,
        query: { page: 1, page_size: 20 },
        items,
        total: items.length,
        page: 1,
        page_size: 20,
      },
    },
  });
}
it("offers board navigation and confirmed deletion, then receives the updated history through SSE", async () => {
  const remove = vi
    .spyOn(api, "deleteSessionHistory")
    .mockResolvedValue(undefined);
  push([record]);
  render(
    <MemoryRouter>
      <LeaseHistory />
    </MemoryRouter>,
  );
  expect(
    screen.queryByRole("button", { name: "修改" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "禁用" }),
  ).not.toBeInTheDocument();
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "更多" }));
  expect(screen.getByRole("menuitem", { name: "查看开发板" })).toHaveAttribute(
    "href",
    "/boards/board%20one",
  );
  await user.keyboard("{Escape}");
  await user.click(screen.getByRole("button", { name: "删除" }));
  expect(remove).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "确认" }));
  expect(remove).toHaveBeenCalledWith(record.session_id);
  act(() => push([]));
  expect(screen.queryByText(record.session_id)).not.toBeInTheDocument();
});
it("does not offer deletion for a session that has not ended", async () => {
  push([{ ...record, ended_at: null, end_reason: null }]);
  render(
    <MemoryRouter>
      <LeaseHistory />
    </MemoryRouter>,
  );
  expect(screen.getByRole("button", { name: "会话详情" })).toBeVisible();
  expect(
    screen.queryByRole("menuitem", { name: "删除" }),
  ).not.toBeInTheDocument();
});

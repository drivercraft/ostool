import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import Audits from "./Audits";
const mocks = vi.hoisted(() => ({
  data: undefined as unknown,
  filters: vi.fn(),
}));
vi.mock("@/api/events", () => ({
  useResource: () => mocks.data,
  useErrors: () => ({}),
  admin: { setAuditFilters: mocks.filters },
}));
vi.mock("@/api/client", () => ({
  api: { auditViewed: vi.fn().mockResolvedValue(undefined) },
}));
beforeEach(() => {
  mocks.data = undefined;
  mocks.filters.mockClear();
  localStorage.clear();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
it("shows table headers and loading placeholders before declaring the audit list empty", () => {
  const view = render(<Audits />);
  expect(screen.getAllByRole("columnheader")).toHaveLength(7);
  expect(screen.getByRole("status").textContent).toBe("正在加载数据");
  expect(screen.queryByText("暂无数据")).toBeNull();
  mocks.data = {
    active: true,
    query: { page: 1, page_size: 20 },
    items: [],
    total: 0,
    page: 1,
    page_size: 20,
    actions: [],
    resources: [],
  };
  view.rerender(<Audits />);
  expect(screen.getByText("暂无数据")).toBeTruthy();
});
it("keeps previous audit rows visible during a slow filter change", () => {
  mocks.data = {
    active: true,
    query: { page: 1, page_size: 20 },
    items: [
      {
        id: "one",
        created_at: "2026-01-01T00:00:00Z",
        actor_name: "Original actor",
        actor_kind: "admin",
        source: "admin",
        summary: "Original action",
        resource: "boards",
        result: "success",
        action: "board.create",
        status_code: 201,
      },
    ],
    total: 1,
    page: 1,
    page_size: 20,
    actions: [],
    resources: [],
  };
  render(<Audits />);
  fireEvent.change(screen.getByLabelText("搜索审计日志"), {
    target: { value: "next filter" },
  });
  expect(screen.getByText("Original actor")).toBeTruthy();
  expect(screen.getByRole("status").textContent).toBe("正在更新数据");
  expect(screen.queryByText("暂无数据")).toBeNull();
});

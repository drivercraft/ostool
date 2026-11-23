import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { afterEach, expect, it, vi } from "vitest";
import { admin } from "@/api/events";
import type { RentalRequest } from "@/types/api";
import RentalRequests from "./RentalRequests";
vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    disconnect() {}
  },
);
afterEach(() => {
  cleanup();
  sessionStorage.clear();
});
function mount(status: RentalRequest["status"]) {
  const request: RentalRequest = {
    id: "request",
    board_id: "board",
    user_id: "user",
    user_email: "user@test.local",
    starts_at: "2026-11-01T00:00:00Z",
    ends_at: "2026-11-01T02:00:00Z",
    notes: "",
    status,
    decision_note: "",
    reviewed_at: null,
    rental_id: null,
    created_at: "2026-11-01T00:00:00Z",
  };
  admin.apply({
    epoch: crypto.randomUUID(),
    revision: 1,
    kind: "snapshot",
    data: {
      listing: {
        active: true,
        query: { view: "rental_requests", page: 1, page_size: 20 },
        items: [request],
        total: 1,
        page: 1,
        page_size: 20,
      },
    },
  });
  render(
    <MemoryRouter>
      <RentalRequests />
    </MemoryRouter>,
  );
}
it.each(["同意", "拒绝"])(
  "shows %s directly and opens its review form",
  async (label) => {
    mount("pending");
    const row = screen.getByRole("row", { name: /user@test.local/ });
    expect(within(row).getAllByRole("button")).toHaveLength(3);
    expect(within(row).getByRole("button", { name: "同意" })).toBeVisible();
    expect(within(row).getByRole("button", { name: "拒绝" })).toBeVisible();
    await userEvent
      .setup()
      .click(within(row).getByRole("button", { name: label }));
    expect(
      screen.getByRole("dialog", { name: `${label}租赁申请` }),
    ).toBeVisible();
  },
);
it("only keeps related navigation for completed requests", () => {
  mount("approved");
  expect(
    screen.queryByRole("button", { name: "同意" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "拒绝" }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "更多" })).toBeVisible();
});

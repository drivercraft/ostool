import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, expect, it, vi } from "vitest";
import { admin } from "@/api/events";
import type { Rental } from "@/types/api";
import Rentals from "./Rentals";
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
it.each([
  ["scheduled", "scheduled", true],
  ["active", "scheduled", true],
  ["expired", "scheduled", false],
  ["cancelled", "scheduled", false],
  ["active", "instant", false],
] as const)(
  "keeps edit and cancel aligned and disables unavailable actions for %s / %s rentals",
  (status, source, editable) => {
    const rental: Rental = {
      id: "rental",
      board_id: "board",
      user_id: "user",
      user_email: "user@test.local",
      starts_at: "2026-11-01T00:00:00Z",
      ends_at: "2026-11-01T02:00:00Z",
      notes: "",
      status,
      source,
      cancelled_at: null,
      created_at: "2026-11-01T00:00:00Z",
    };
    admin.apply({
      epoch: crypto.randomUUID(),
      revision: 1,
      kind: "snapshot",
      data: {
        boards: [],
        listing: {
          active: true,
          query: { view: "rentals", page: 1, page_size: 20 },
          items: [rental],
          total: 1,
          page: 1,
          page_size: 20,
        },
      },
    });
    render(
      <MemoryRouter>
        <Rentals />
      </MemoryRouter>,
    );
    const edit = screen.getByRole(editable ? "link" : "button", {
      name: "编辑",
    });
    if (editable) expect(edit).toHaveAttribute("href", "/rentals/rental");
    else expect(edit).toBeDisabled();
    const cancel = screen.getByRole("button", { name: "取消租赁" });
    if (status === "expired" || status === "cancelled")
      expect(cancel).toBeDisabled();
    else expect(cancel).toBeEnabled();
    expect(screen.getByRole("button", { name: "更多" })).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "禁用" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "启用" }),
    ).not.toBeInTheDocument();
  },
);

import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, renderHook } from "@testing-library/react";
import { admin } from "@/api/events";
import { useServerList } from "./server-list";
import type { ListPage, ManagedUser } from "@/types/api";
afterEach(() => {
  cleanup();
  admin.setListingFilters(null);
  admin.reset();
  vi.restoreAllMocks();
  vi.useRealTimers();
});
it("debounces search, retains the last page, and replaces it only when new data arrives", () => {
  vi.useFakeTimers();
  admin.reset();
  const filters = vi.spyOn(admin, "setListingFilters");
  const hook = renderHook(({ q }) => useServerList("users", { q }), {
    initialProps: { q: "" },
  });
  expect(hook.result.current.loading).toBe(true);
  expect(filters).toHaveBeenCalledTimes(1);
  const page = (q: string, id: string): ListPage<ManagedUser> => ({
    active: true,
    query: { view: "users", q, page: 1, page_size: 20 },
    items: [
      {
        id,
        username: id,
        email: id + "@example.com",
        display_name: null,
        role_id: "member",
        disabled: false,
        created_at: "2026-01-01T00:00:00Z",
      },
    ],
    total: 1,
    page: 1,
    page_size: 20,
  });
  act(() =>
    admin.apply({
      epoch: "e",
      revision: 1,
      kind: "snapshot",
      data: { listing: page("", "previous") },
    }),
  );
  expect(hook.result.current.loading).toBe(false);
  hook.rerender({ q: "n" });
  hook.rerender({ q: "ne" });
  hook.rerender({ q: "new" });
  expect(hook.result.current.rows[0].id).toBe("previous");
  expect(hook.result.current.updating).toBe(true);
  act(() => vi.advanceTimersByTime(299));
  expect(filters).toHaveBeenCalledTimes(1);
  act(() => vi.advanceTimersByTime(1));
  expect(filters).toHaveBeenLastCalledWith({
    view: "users",
    q: "new",
    page: 1,
    page_size: 20,
  });
  act(() =>
    admin.apply({
      epoch: "e",
      revision: 2,
      kind: "update",
      data: { listing: page("new", "replacement") },
    }),
  );
  expect(hook.result.current.rows[0].id).toBe("replacement");
  expect(hook.result.current.updating).toBe(false);
});

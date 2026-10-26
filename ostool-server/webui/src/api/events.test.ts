import { afterEach, describe, it, expect, vi } from "vitest";
import { AdminStore } from "./events";
import * as client from "./client";
import type { BoardConfig } from "@/types/api";
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  vi.useRealTimers();
});
describe("management event projection", () => {
  it("waits for fresh data after reconnecting and preserves the previous projection", () => {
    const source = new (class extends EventTarget {
      onopen: ((event: Event) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      close = vi.fn();
    })();
    vi.stubGlobal(
      "EventSource",
      Object.assign(
        vi.fn(function () {
          return source;
        }),
        { CLOSED: 2 },
      ),
    );
    const store = new AdminStore();
    store.connect();
    source.onopen?.(new Event("open"));
    expect(store.getConnection()).toBe("connecting");
    const snapshot = (epoch: string) =>
      source.dispatchEvent(
        new MessageEvent("snapshot", {
          data: JSON.stringify({
            epoch,
            revision: 1,
            kind: "snapshot",
            data: { boards: [] },
          }),
        }),
      );
    snapshot("first");
    expect(store.getConnection()).toBe("connected");
    const previous = store.get("boards");
    source.onerror?.(new Event("error"));
    expect(store.getConnection()).toBe("reconnecting");
    source.onopen?.(new Event("open"));
    expect(store.getConnection()).toBe("reconnecting");
    expect(store.get("boards")).toBe(previous);
    snapshot("recovered");
    expect(store.getConnection()).toBe("connected");
    store.disconnect();
  });
  it("retries an authentication outage without losing data and cancels retries on teardown", async () => {
    vi.useFakeTimers();
    class Source extends EventTarget {
      static CLOSED = 2;
      readyState = 1;
      onerror: (() => void) | null = null;
      close = vi.fn();
      constructor() {
        super();
        sources.push(this);
      }
    }
    const sources: Source[] = [];
    vi.stubGlobal("EventSource", Source);
    vi.spyOn(client, "isPublicEdition").mockReturnValue(true);
    vi.spyOn(client, "refreshAuth").mockRejectedValue(
      new client.ApiRequestError("restarting", "service_unavailable", 503),
    );
    const store = new AdminStore();
    store.connect();
    store.apply({
      epoch: "e",
      revision: 1,
      kind: "snapshot",
      data: { boards: [] },
    });
    const previous = store.get("boards");
    sources[0].dispatchEvent(new Event("auth-expired"));
    await vi.advanceTimersByTimeAsync(500);
    expect(store.getConnection()).toBe("reconnecting");
    expect(store.get("boards")).toBe(previous);
    await vi.advanceTimersByTimeAsync(500);
    expect(sources).toHaveLength(2);
    sources[1].dispatchEvent(
      new MessageEvent("snapshot", {
        data: JSON.stringify({
          epoch: "new",
          revision: 1,
          kind: "snapshot",
          data: { boards: [] },
        }),
      }),
    );
    expect(store.getConnection()).toBe("connected");
    sources[1].dispatchEvent(new Event("auth-expired"));
    await vi.advanceTimersByTimeAsync(500);
    store.disconnect();
    await vi.advanceTimersByTimeAsync(30000);
    expect(sources).toHaveLength(2);
  });

  it("reauthorizes a permanently closed stream before reconnecting", async () => {
    const sources: EventTarget[] = [];
    class Source extends EventTarget {
      static CLOSED = 2;
      readyState = 2;
      onerror: (() => void) | null = null;
      close = vi.fn();
      constructor() {
        super();
        sources.push(this);
      }
    }
    vi.stubGlobal("EventSource", Source);
    vi.spyOn(client, "isPublicEdition").mockReturnValue(true);
    const me = vi.spyOn(client.api, "me").mockResolvedValue({
      id: "a",
      username: "admin",
      email: "admin@test.local",
      display_name: null,
      is_admin: true,
    });
    const store = new AdminStore();
    store.connect();
    (sources[0] as Source).onerror?.();
    await Promise.resolve();
    await Promise.resolve();
    expect(me).toHaveBeenCalledOnce();
    expect(sources).toHaveLength(2);
    store.disconnect();
  });
  it("reconciles rows by ID, removes deleted rows, and preserves unchanged references", () => {
    const store = new AdminStore();
    const a = { id: "a", board_type: "arm" } as BoardConfig,
      b = { id: "b" } as BoardConfig;
    store.apply({
      epoch: "e",
      revision: 1,
      kind: "snapshot",
      data: { boards: [a, b] },
    });
    const original = store.get("boards")!;
    store.apply({
      epoch: "e",
      revision: 2,
      kind: "update",
      data: { boards: [{ ...a }] },
    });
    expect(store.get("boards")).toHaveLength(1);
    expect(store.get("boards")![0]).toBe(original[0]);
    store.apply({
      epoch: "e",
      revision: 1,
      kind: "update",
      data: { boards: [] },
    });
    expect(store.get("boards")).toHaveLength(1);
  });
  it("resets the subscription on a revision gap without clearing displayed data", () => {
    const store = new AdminStore();
    store.apply({
      epoch: "e",
      revision: 1,
      kind: "snapshot",
      data: { boards: [] },
    });
    const restart = vi.spyOn(store, "restart").mockImplementation(() => {});
    store.apply({
      epoch: "e",
      revision: 3,
      kind: "update",
      data: { boards: [{ id: "lost" } as BoardConfig] },
    });
    expect(restart).toHaveBeenCalledOnce();
    expect(store.get("boards")).toEqual([]);
  });
  it("recovers with a new epoch snapshot and exposes per-topic errors without deleting last good data", () => {
    const store = new AdminStore();
    store.apply({
      epoch: "e",
      revision: 4,
      kind: "snapshot",
      data: { boards: [] },
    });
    store.apply({
      epoch: "new",
      revision: 1,
      kind: "snapshot",
      data: { boards: [{ id: "new" } as BoardConfig] },
    });
    store.apply({
      epoch: "new",
      revision: 2,
      kind: "update",
      data: { boards: { error: "unavailable" } as never },
    });
    expect(store.get("boards")![0].id).toBe("new");
    expect(store.getErrors().boards).toBe("unavailable");
    store.apply({
      epoch: "new",
      revision: 3,
      kind: "update",
      data: { boards: [] },
    });
    expect(store.getErrors()).toEqual({});
  });
});

it("subscribes to bounded lease history filters and preserves projections when changing pages", () => {
  const urls: string[] = [];
  const close = vi.fn();
  vi.stubGlobal(
    "EventSource",
    class extends EventTarget {
      close = close;
      onerror = null;
      constructor(url: string) {
        super();
        urls.push(url);
      }
    },
  );
  vi.spyOn(client, "isPublicEdition").mockReturnValue(false);
  const store = new AdminStore();
  store.connect();
  store.apply({
    epoch: "history",
    revision: 1,
    kind: "snapshot",
    data: { boards: [] },
  });
  const old = store.get("boards");
  store.setLeaseFilters({
    board: "device/one",
    q: "user_%",
    page: 2,
    page_size: 10,
  });
  const query = new URL(urls.at(-1)!, "http://localhost").searchParams;
  expect(query.get("lease_view")).toBe("1");
  expect(query.get("lease_board")).toBe("device/one");
  expect(query.get("lease_q")).toBe("user_%");
  expect(query.get("lease_page")).toBe("2");
  expect(store.get("boards")).toBe(old);
  expect(close).toHaveBeenCalledTimes(1);
  store.setLeaseFilters(null);
  expect(urls.at(-1)).toBe("/api/v1/admin/events");
  store.disconnect();
});

it("discards selected editor details when changing boards or leaving the editor", () => {
  const store = new AdminStore();
  store.setBoardEditor("one");
  store.apply({
    epoch: "private",
    revision: 1,
    kind: "snapshot",
    data: {
      board_editor: {
        id: "one",
        board: { id: "one", reset_command: "private command" } as BoardConfig,
      },
    },
  });
  expect(store.get("board_editor")?.board?.reset_command).toBe(
    "private command",
  );
  store.setBoardEditor("two");
  expect(store.get("board_editor")).toBeUndefined();
  store.apply({
    epoch: "private",
    revision: 2,
    kind: "update",
    data: { board_editor: { id: "two", board: { id: "two" } as BoardConfig } },
  });
  expect(store.get("board_editor")?.id).toBe("two");
  store.setBoardEditor(null);
  expect(store.get("board_editor")).toBeUndefined();
});

import { afterEach, describe, expect, it, vi } from "vitest";

import { api } from ".";

describe("api", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("accepts an empty 202 response when deleting a session", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(null, {
          status: 202,
        }),
      ),
    );

    await expect(api.deleteSession("demo-session")).resolves.toBeUndefined();
  });

  it("uses REST endpoints for admin session updates and close actions", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        Response.json({
          session: {
            id: "demo/session",
            board_id: "board-1",
            client_name: "web-ui",
            source_ip: "127.0.0.1",
            state: "active",
            created_at: "2026-01-01T00:00:00Z",
            last_heartbeat_at: "2026-01-01T00:00:00Z",
            expires_at: "2026-01-01T00:01:00Z",
            ended_at: null,
            failure_message: null,
          },
          lease: null,
          user_id: null,
          source_ip: "127.0.0.1",
        }),
      )
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.updateSession("demo/session", {
      client_name: "web-ui",
      failure_message: null,
    })).resolves.toMatchObject({ session: { id: "demo/session" } });
    await expect(api.closeSession("demo/session")).resolves.toBeUndefined();

    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      "/api/v1/admin/sessions/demo%2Fsession",
      expect.objectContaining({
        method: "PUT",
        body: JSON.stringify({ client_name: "web-ui", failure_message: null }),
      }),
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      "/api/v1/admin/sessions/demo%2Fsession/close",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("uses REST resource endpoints for admin user read and delete", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        Response.json({
          id: "u/1",
          username: "alice",
          display_name: "Alice",
          nickname: null,
          avatar_url: null,
          email: "alice@example.com",
          phone: null,
          department: null,
          title: null,
          disabled: false,
          last_login_at: null,
          created_at: "2026-01-01T00:00:00Z",
          updated_at: "2026-01-01T00:00:00Z",
        }),
      )
      .mockResolvedValueOnce(
        new Response(null, {
          status: 204,
        }),
      );
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.getAdminUser("u/1")).resolves.toMatchObject({
      id: "u/1",
      username: "alice",
    });
    await expect(api.deleteAdminUser("u/1")).resolves.toBeUndefined();

    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      "/api/v1/admin/users/u%2F1",
      expect.objectContaining({
        credentials: "same-origin",
      }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/v1/admin/users/u%2F1",
      expect.objectContaining({
        method: "DELETE",
        credentials: "same-origin",
      }),
    );
  });

  it("reports a service connection hint when fetch fails", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));

    await expect(api.getOverview()).rejects.toThrow(
      "无法连接 ostool-server，服务可能正在安装、升级或重启，请稍后刷新页面。",
    );
  });
});

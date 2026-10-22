import { afterEach, describe, expect, it, vi } from "vitest";

import { api, refreshAuth, ApiRequestError } from "./client";

describe("api client", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("preserves the login on refresh outages and clears it only after confirmed rejection", async () => {
    window.history.replaceState({}, "", "/admin/overview");
    const key = "ostool.admin.session-state";
    localStorage.setItem(key, "signed-in");
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(new Response(null, { status: 503 }))
      .mockRejectedValueOnce(new TypeError("network interrupted"))
      .mockResolvedValueOnce(new Response(null, { status: 200 }))
      .mockResolvedValueOnce(new Response(null, { status: 401 }));
    vi.stubGlobal("fetch", fetch);
    await expect(refreshAuth()).rejects.toMatchObject({ status: 503 });
    expect(localStorage.getItem(key)).toBe("signed-in");
    await expect(refreshAuth()).rejects.toThrow("network interrupted");
    expect(localStorage.getItem(key)).toBe("signed-in");
    await expect(refreshAuth()).resolves.toBeUndefined();
    expect(localStorage.getItem(key)).toBe("signed-in");
    await expect(refreshAuth()).rejects.toBeInstanceOf(ApiRequestError);
    expect(localStorage.getItem(key)).toBeNull();
    window.history.replaceState({}, "", "/");
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

  it("deletes a loader image using its encoded digest path", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetch);

    await expect(
      api.deleteLoaderImage("digest/value"),
    ).resolves.toBeUndefined();
    expect(fetch).toHaveBeenCalledWith(
      "/api/v1/admin/loader-images/digest%2Fvalue",
      expect.objectContaining({ method: "DELETE" }),
    );
  });

  it("reports a service connection hint when fetch fails", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockRejectedValue(new TypeError("Failed to fetch")),
    );

    await expect(api.me()).rejects.toThrow(
      "无法连接 ostool-server，服务可能正在安装、升级或重启，连接恢复后将自动同步状态。",
    );
  });
});

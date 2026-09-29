import { afterEach, describe, expect, it, vi } from "vitest";

import { api } from "./client";

describe("api client", () => {
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

  it("deletes a loader image using its encoded digest path", async () => {
    const fetch = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetch);

    await expect(api.deleteLoaderImage("digest/value")).resolves.toBeUndefined();
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

    await expect(api.getOverview()).rejects.toThrow(
      "无法连接 ostool-server，服务可能正在安装、升级或重启，连接恢复后将自动同步状态。",
    );
  });
});

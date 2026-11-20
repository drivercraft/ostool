import { afterEach, describe, expect, it, vi } from "vitest";

import { api, refreshAuth, ApiRequestError } from "./client";

class UploadRequest {
  static requests: UploadRequest[] = [];
  upload = {
    onprogress: undefined as ((event: ProgressEvent) => void) | undefined,
  };
  open = vi.fn();
  setRequestHeader = vi.fn();
  getResponseHeader = () => "application/json";
  send = vi.fn();
  withCredentials = false;
  status = 200;
  statusText = "OK";
  responseText = "";
  onload?: () => void;
  onerror?: () => void;
  constructor() {
    UploadRequest.requests.push(this);
  }
  complete(status: number, body: unknown) {
    this.status = status;
    this.responseText = JSON.stringify(body);
    this.onload?.();
  }
}

describe("api client", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("preserves the login on refresh outages and clears it only after confirmed rejection", async () => {
    window.history.replaceState({}, "", "/qcl/overview");
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

  it("keeps normal deletion compatible and requests forced cleanup only when explicitly selected", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetch);
    await api.deleteSession("stuck/id", true);
    await api.deleteBoard("board/id", true);
    await api.deleteSession("normal");
    expect(fetch).toHaveBeenNthCalledWith(
      1,
      "/api/v1/admin/sessions/stuck%2Fid?force=true",
      expect.objectContaining({ method: "DELETE" }),
    );
    expect(fetch).toHaveBeenNthCalledWith(
      2,
      "/api/v1/admin/boards/board%2Fid?force=true",
      expect.objectContaining({ method: "DELETE" }),
    );
    expect(fetch).toHaveBeenNthCalledWith(
      3,
      "/api/v1/admin/sessions/normal",
      expect.objectContaining({ method: "DELETE" }),
    );
  });

  it("reports admin errors directly without opening identity confirmation", async () => {
    window.history.replaceState({}, "", "/qcl/overview");
    const verify = vi.fn();
    window.addEventListener("ostool:step-up", verify);
    const fetch = vi
      .fn()
      .mockResolvedValue(
        Response.json(
          { code: "reauth_required", message: "verification rejected" },
          { status: 403 },
        ),
      );
    vi.stubGlobal("fetch", fetch);
    try {
      await expect(api.deleteSession("demo-session")).rejects.toMatchObject({
        code: "reauth_required",
        status: 403,
      });
      expect(verify).not.toHaveBeenCalled();
      expect(fetch).toHaveBeenCalledTimes(1);
    } finally {
      window.removeEventListener("ostool:step-up", verify);
      window.history.replaceState({}, "", "/");
    }
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

  it("updates only disabled flags through the partial status endpoints", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetch);
    await api.setBoardDisabled("board/id", true);
    await api.setUserDisabled("user-id", false);
    expect(fetch).toHaveBeenNthCalledWith(
      1,
      "/api/v1/admin/boards/board%2Fid",
      expect.objectContaining({ method: "PATCH", body: '{"disabled":true}' }),
    );
    expect(fetch).toHaveBeenNthCalledWith(
      2,
      "/api/v1/admin/users/user-id",
      expect.objectContaining({ method: "PATCH", body: '{"disabled":false}' }),
    );
  });

  it("uploads with byte progress and preserves the admin surface, cookies and refresh retry", async () => {
    window.history.replaceState({}, "", "/qcl/boards/demo");
    UploadRequest.requests = [];
    vi.stubGlobal("XMLHttpRequest", UploadRequest);
    const refresh = vi
      .fn()
      .mockResolvedValue(new Response(null, { status: 200 }));
    vi.stubGlobal("fetch", refresh);
    const progress = vi.fn();
    const file = new File(["efi"], "loader.efi");
    const result = api.uploadLoaderImage(file, "loader.efi", progress);
    const first = UploadRequest.requests[0];
    expect(first.open).toHaveBeenCalledWith(
      "POST",
      "/api/v1/admin/loader-images",
    );
    expect(first.withCredentials).toBe(true);
    expect(first.setRequestHeader).toHaveBeenCalledWith(
      "x-ostool-surface",
      "admin",
    );
    expect(first.setRequestHeader).toHaveBeenCalledWith(
      "x-image-version",
      "loader.efi",
    );
    first.upload.onprogress?.(
      new ProgressEvent("progress", {
        loaded: 2,
        total: 3,
        lengthComputable: true,
      }),
    );
    expect(progress).toHaveBeenLastCalledWith(2, 3);
    first.complete(401, { code: "unauthorized", message: "expired" });
    await vi.waitFor(() => expect(UploadRequest.requests).toHaveLength(2));
    const retry = UploadRequest.requests[1];
    expect(retry.send).toHaveBeenCalledWith(file);
    const image = { sha256: "digest", size: 3, version: "loader.efi" };
    retry.complete(201, image);
    await expect(result).resolves.toEqual(image);
    expect(refresh).toHaveBeenCalledWith(
      "/api/v1/auth/refresh",
      expect.any(Object),
    );
    window.history.replaceState({}, "", "/");
  });

  it("reports upload network failures through the standard error path", async () => {
    UploadRequest.requests = [];
    vi.stubGlobal("XMLHttpRequest", UploadRequest);
    const result = api.uploadLoaderImage(
      new File(["efi"], "loader.efi"),
      undefined,
      vi.fn(),
    );
    const rejected = expect(result).rejects.toThrow("无法连接 ostool-server");
    UploadRequest.requests[0].onerror?.();
    await rejected;
  });
});

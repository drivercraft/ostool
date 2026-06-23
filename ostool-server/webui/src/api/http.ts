import type { ErrorResponse } from "@/types/api";

export type RequestOptions = RequestInit & {
  bodyJson?: unknown;
};

const NETWORK_ERROR_MESSAGE =
  "无法连接 ostool-server，服务可能正在安装、升级或重启，请稍后刷新页面。";

async function readJsonBody<T>(response: Response): Promise<T | undefined> {
  const text = await response.text();
  if (!text.trim()) {
    return undefined;
  }
  const contentType = response.headers.get("content-type") ?? "";
  if (!contentType.includes("application/json") && text.trimStart().startsWith("<")) {
    throw new Error(NETWORK_ERROR_MESSAGE);
  }
  try {
    return JSON.parse(text) as T;
  } catch {
    throw new Error(NETWORK_ERROR_MESSAGE);
  }
}

export async function request<T>(
  path: string,
  options: RequestOptions = {},
): Promise<T> {
  const headers = new Headers(options.headers);
  let body = options.body;

  if (options.bodyJson !== undefined) {
    headers.set("content-type", "application/json");
    body = JSON.stringify(options.bodyJson);
  }

  const response = await fetch(path, {
    ...options,
    credentials: "same-origin",
    headers,
    body,
  }).catch(() => {
    throw new Error(NETWORK_ERROR_MESSAGE);
  });

  if (!response.ok) {
    const error = (await readJsonBody<ErrorResponse>(response).catch(() => null)) ?? null;
    throw new Error(error?.message || `请求失败：${response.status}`);
  }

  return (await readJsonBody<T>(response)) as T;
}

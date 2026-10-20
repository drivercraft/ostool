import type {
  AdminBoardUpsertRequest,
  AdminOverviewResponse,
  AdminServerConfigResponse,
  AdminSessionsResponse,
  AdminTftpConfigResponse,
  AdminTftpStatusResponse,
  BoardConfig,
  DtbFileResponse,
  ErrorResponse,
  LoaderDeviceSummary,
  LoaderImage,
  LoaderUpdateJob,
  NetworkInterfaceSummary,
  SerialPortSummary,
  TftpConfig,
  UpdateServerConfigRequest,
  VirtualDeviceSummary,
  VirtualDevicesResponse,
} from "@/types/api";

type RequestOptions = RequestInit & {
  bodyJson?: unknown;
};

const NETWORK_ERROR_MESSAGE =
  "无法连接 ostool-server，服务可能正在安装、升级或重启，连接恢复后将自动同步状态。";

async function readJsonBody<T>(response: Response): Promise<T | undefined> {
  const text = await response.text();
  if (!text.trim()) {
    return undefined;
  }
  return JSON.parse(text) as T;
}

async function request<T>(
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
    headers: (() => {
      const accessToken = localStorage.getItem("ostool.access_token");
      if (accessToken) headers.set("authorization", `Bearer ${accessToken}`);
      return headers;
    })(),
    body,
  }).catch(() => {
    throw new Error(NETWORK_ERROR_MESSAGE);
  });

  if (!response.ok) {
    const error =
      (await readJsonBody<ErrorResponse>(response).catch(() => null)) ?? null;
    throw new Error(error?.message || `请求失败：${response.status}`);
  }

  return (await readJsonBody<T>(response)) as T;
}

export const api = {
  login(email: string, password: string) {
    return request<AuthToken>("/api/v1/auth/login", {
      method: "POST",
      bodyJson: { email, password },
    });
  },
  register(email: string, password: string, displayName?: string) {
    return request<AuthToken>("/api/v1/auth/register", {
      method: "POST",
      bodyJson: { email, password, display_name: displayName || null },
    });
  },
  authorizeDevice(userCode: string) {
    return request<void>("/api/v1/auth/device/authorize", {
      method: "POST",
      bodyJson: { user_code: userCode },
    });
  },
  me() {
    return request<AuthUser>("/api/v1/auth/me");
  },
  createPersonalAccessToken(name: string, expiresInDays?: number) {
    return request<PersonalAccessToken>("/api/v1/auth/personal-access-tokens", {
      method: "POST",
      bodyJson: { name, expires_in_days: expiresInDays || null },
    });
  },
  revokePersonalAccessToken(id: string) {
    return request<void>(`/api/v1/auth/personal-access-tokens/${encodeURIComponent(id)}`, { method: "DELETE" });
  },
  uploadLoaderImage(file: File, version?: string) {
    return request<LoaderImage>("/api/v1/admin/loader-images", {
      method: "POST",
      headers: version ? { "X-Image-Version": version } : undefined,
      body: file,
    });
  },
  deleteLoaderImage(imageSha256: string) {
    return request<void>(
      `/api/v1/admin/loader-images/${encodeURIComponent(imageSha256)}`,
      { method: "DELETE" },
    );
  },
  queueLoaderUpdate(boardId: string, imageSha256: string) {
    return request<LoaderUpdateJob>(
      `/api/v1/admin/boards/${encodeURIComponent(boardId)}/loader-updates`,
      { method: "POST", bodyJson: { image_sha256: imageSha256 } },
    );
  },
  cancelLoaderUpdate(boardId: string, updateId: string) {
    return request<LoaderUpdateJob>(
      `/api/v1/admin/boards/${encodeURIComponent(boardId)}/loader-updates/${encodeURIComponent(updateId)}`,
      { method: "DELETE" },
    );
  },
  powerAction(payload: {
    request_id: string;
    action: "on" | "off";
    power_management: import("@/types/api").PowerManagementConfig;
  }) {
    return request<import("./events").PowerResult>(
      "/api/v1/admin/power-actions",
      { method: "POST", bodyJson: payload },
    );
  },
  getPowerAction(id: string) {
    return request<import("./events").PowerResult>(
      `/api/v1/admin/power-actions/${encodeURIComponent(id)}`,
    );
  },
  getOverview() {
    return request<AdminOverviewResponse>("/api/v1/admin/overview");
  },
  listBoards() {
    return request<BoardConfig[]>("/api/v1/admin/boards");
  },
  listLoaderDevices() {
    return request<LoaderDeviceSummary[]>("/api/v1/admin/loader-devices");
  },
  listVirtualDevices() {
    return request<VirtualDevicesResponse>("/api/v1/admin/virtual-devices");
  },
  createVirtualDevice(macAddress?: string) {
    return request<VirtualDeviceSummary>("/api/v1/admin/virtual-devices", {
      method: "POST",
      bodyJson: { mac_address: macAddress || null },
    });
  },
  deleteVirtualDevice(deviceId: string) {
    return request<void>(
      `/api/v1/admin/virtual-devices/${encodeURIComponent(deviceId)}`,
      {
        method: "DELETE",
      },
    );
  },
  getBoard(boardId: string) {
    return request<BoardConfig>(
      `/api/v1/admin/boards/${encodeURIComponent(boardId)}`,
    );
  },
  listDtbs() {
    return request<DtbFileResponse[]>("/api/v1/admin/dtbs");
  },
  getDtb(dtbName: string) {
    return request<DtbFileResponse>(
      `/api/v1/admin/dtbs/${encodeURIComponent(dtbName)}`,
    );
  },
  createDtb(dtbName: string, file: Blob) {
    return request<DtbFileResponse>("/api/v1/admin/dtbs", {
      method: "POST",
      headers: {
        "X-Dtb-Name": dtbName,
      },
      body: file,
    });
  },
  updateDtb(currentName: string, nextName?: string | null, file?: Blob | null) {
    const headers = new Headers();
    if (nextName) {
      headers.set("X-Dtb-Name", nextName);
    }
    return request<DtbFileResponse>(
      `/api/v1/admin/dtbs/${encodeURIComponent(currentName)}`,
      {
        method: "PUT",
        headers,
        body: file ?? undefined,
      },
    );
  },
  deleteDtb(dtbName: string) {
    return request<void>(`/api/v1/admin/dtbs/${encodeURIComponent(dtbName)}`, {
      method: "DELETE",
    });
  },
  listSerialPorts() {
    return request<SerialPortSummary[]>("/api/v1/admin/serial-ports");
  },
  listNetworkInterfaces() {
    return request<NetworkInterfaceSummary[]>(
      "/api/v1/admin/network-interfaces",
    );
  },
  createBoard(payload: AdminBoardUpsertRequest) {
    return request<BoardConfig>("/api/v1/admin/boards", {
      method: "POST",
      bodyJson: payload,
    });
  },
  updateBoard(boardId: string, payload: AdminBoardUpsertRequest) {
    return request<BoardConfig>(
      `/api/v1/admin/boards/${encodeURIComponent(boardId)}`,
      {
        method: "PUT",
        bodyJson: payload,
      },
    );
  },
  deleteBoard(boardId: string) {
    return request<void>(
      `/api/v1/admin/boards/${encodeURIComponent(boardId)}`,
      {
        method: "DELETE",
      },
    );
  },
  listSessions() {
    return request<AdminSessionsResponse>("/api/v1/admin/sessions");
  },
  deleteSession(sessionId: string) {
    return request<void>(
      `/api/v1/admin/sessions/${encodeURIComponent(sessionId)}`,
      {
        method: "DELETE",
      },
    );
  },
  getTftpConfig() {
    return request<AdminTftpConfigResponse>("/api/v1/admin/tftp");
  },
  updateTftpConfig(tftp: TftpConfig) {
    return request<AdminTftpConfigResponse>("/api/v1/admin/tftp", {
      method: "PUT",
      bodyJson: tftp,
    });
  },
  getTftpStatus() {
    return request<AdminTftpStatusResponse>("/api/v1/admin/tftp/status");
  },
  reconcileTftp() {
    return request<AdminTftpStatusResponse>("/api/v1/admin/tftp/reconcile", {
      method: "POST",
    });
  },
  getServerConfig() {
    return request<AdminServerConfigResponse>("/api/v1/admin/server-config");
  },
  updateServerConfig(payload: UpdateServerConfigRequest) {
    return request<AdminServerConfigResponse>("/api/v1/admin/server-config", {
      method: "PUT",
      bodyJson: payload,
    });
  },
};

export interface AuthToken {
  access_token: string;
  refresh_token: string;
  token_type: string;
  expires_in: number;
  scope: string;
}

export interface AuthUser {
  id: string;
  email: string;
  display_name: string | null;
  is_admin: boolean;
}

export interface PersonalAccessToken {
  id: string;
  token: string;
  name: string;
  expires_at: string | null;
}

export function saveAuthToken(token: AuthToken) {
  localStorage.setItem("ostool.access_token", token.access_token);
  localStorage.setItem("ostool.refresh_token", token.refresh_token);
}

export function clearAuthToken() {
  localStorage.removeItem("ostool.access_token");
  localStorage.removeItem("ostool.refresh_token");
}

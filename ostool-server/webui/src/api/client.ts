import { accessToken, refreshToken, authKeys } from "@/auth-session";
import type { UserInput, RoleInput } from "@/types/api";
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

let runtimeEdition: "local" | "public" = "local";
export const isPublicEdition = () => runtimeEdition === "public";
let refreshing: Promise<void> | undefined;
export async function refreshAuth() {
  if (!refreshing)
    refreshing = (async () => {
      const refresh = refreshToken();
      if (!refresh) throw new Error("请重新登录");
      const response = await fetch("/oauth/token", {
        method: "POST",
        headers: { "content-type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({
          grant_type: "refresh_token",
          refresh_token: refresh,
        }),
      });
      if (!response.ok) {
        clearAuthToken();
        throw new Error("登录已过期，请重新登录");
      }
      saveAuthToken(await response.json());
    })().finally(() => {
      refreshing = undefined;
    });
  return refreshing;
}

async function requestResponse(
  path: string,
  options: RequestOptions = {},
): Promise<Response> {
  const headers = new Headers(options.headers);
  let body = options.body;

  if (options.bodyJson !== undefined) {
    headers.set("content-type", "application/json");
    body = JSON.stringify(options.bodyJson);
  }

  let response = await fetch(path, {
    ...options,
    headers: (() => {
      const credential = accessToken();
      if (isPublicEdition() && credential)
        headers.set("authorization", `Bearer ${credential}`);
      return headers;
    })(),
    body,
  }).catch(() => {
    throw new Error(NETWORK_ERROR_MESSAGE);
  });

  if (
    isPublicEdition() &&
    response.status === 401 &&
    refreshToken() &&
    ![
      "/api/v1/auth/login",
      "/api/v1/auth/register",
      "/api/v1/admin/auth/login",
    ].includes(path)
  ) {
    await refreshAuth();
    headers.set("authorization", `Bearer ${accessToken()}`);
    response = await fetch(path, { ...options, headers, body });
  }
  if (!response.ok) {
    const error =
      (await readJsonBody<ErrorResponse>(response).catch(() => null)) ?? null;
    throw new Error(error?.message || `请求失败：${response.status}`);
  }

  return response;
}
async function request<T>(
  path: string,
  options: RequestOptions = {},
): Promise<T> {
  return (await readJsonBody<T>(await requestResponse(path, options))) as T;
}

export type AuthConfig = {
  edition: "local" | "public";
  enabled: boolean;
  allow_registration: boolean;
  database_available: boolean;
};
let authConfigRequest: Promise<AuthConfig> | undefined;
export const api = {
  auditViewed() {
    return request<void>("/api/v1/admin/audits/view", { method: "POST" });
  },
  auditDetail(id: string) {
    return request<import("@/types/api").AuditEvent>(
      `/api/v1/admin/audits/${encodeURIComponent(id)}`,
    );
  },
  async exportAudit(filters: import("@/types/api").AuditFilters) {
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries(filters)) {
      if (value !== undefined && value !== "") params.set(key, String(value));
    }
    return (
      await requestResponse(`/api/v1/admin/audits/export?${params}`)
    ).blob();
  },
  cleanupAudit(keep_days: number) {
    return request<{ deleted: number }>("/api/v1/admin/audits/cleanup", {
      method: "POST",
      bodyJson: { keep_days },
    });
  },
  updateProfile(payload: {
    email: string;
    display_name: string;
    current_password?: string;
  }) {
    return request<void>("/api/v1/account/profile", {
      method: "PUT",
      bodyJson: payload,
    });
  },
  changePassword(current_password: string, new_password: string) {
    return request<void>("/api/v1/account/password", {
      method: "POST",
      bodyJson: { current_password, new_password },
    });
  },
  applyRental(payload: import("@/types/api").RentalApplication) {
    return request<void>("/api/v1/account/rental-requests", {
      method: "POST",
      bodyJson: payload,
    });
  },
  withdrawRentalRequest(id: string) {
    return request<void>(
      `/api/v1/account/rental-requests/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    );
  },
  reviewRentalRequest(
    id: string,
    decision: "approve" | "reject",
    note: string,
  ) {
    return request<void>(
      `/api/v1/admin/rental-requests/${encodeURIComponent(id)}/review`,
      { method: "POST", bodyJson: { decision, note } },
    );
  },
  createRental(payload: import("@/types/api").RentalInput) {
    return request<void>("/api/v1/admin/rentals", {
      method: "POST",
      bodyJson: payload,
    });
  },
  updateRental(id: string, payload: import("@/types/api").RentalInput) {
    return request<void>(`/api/v1/admin/rentals/${id}`, {
      method: "PUT",
      bodyJson: payload,
    });
  },
  cancelRental(id: string) {
    return request<void>(`/api/v1/admin/rentals/${id}`, { method: "DELETE" });
  },
  authConfig() {
    // StrictMode mounts effects twice; share the in-flight bootstrap request.
    return (authConfigRequest ??= request<AuthConfig>("/api/v1/auth/config")
      .then((config) => {
        runtimeEdition = config.edition;
        return config;
      })
      .finally(() => {
        authConfigRequest = undefined;
      }));
  },
  createUser(payload: UserInput) {
    return request<void>("/api/v1/admin/users", {
      method: "POST",
      bodyJson: payload,
    });
  },
  updateUser(id: string, payload: UserInput) {
    return request<void>(`/api/v1/admin/users/${id}`, {
      method: "PUT",
      bodyJson: payload,
    });
  },
  deleteUser(id: string) {
    return request<void>(`/api/v1/admin/users/${id}`, { method: "DELETE" });
  },
  createRole(payload: RoleInput) {
    return request<void>("/api/v1/admin/roles", {
      method: "POST",
      bodyJson: payload,
    });
  },
  updateRole(id: string, payload: RoleInput) {
    return request<void>(`/api/v1/admin/roles/${id}`, {
      method: "PUT",
      bodyJson: payload,
    });
  },
  deleteRole(id: string) {
    return request<void>(`/api/v1/admin/roles/${id}`, { method: "DELETE" });
  },
  async logout() {
    const refresh = refreshToken();
    try {
      if (refresh)
        await fetch("/oauth/revoke", {
          method: "POST",
          headers: { "content-type": "application/x-www-form-urlencoded" },
          body: new URLSearchParams({ token: refresh }),
        });
    } finally {
      clearAuthToken();
    }
  },
  adminLogin(email: string, password: string) {
    return request<AuthToken>("/api/v1/admin/auth/login", {
      method: "POST",
      bodyJson: { email, password },
    });
  },
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
    return request<void>(
      `/api/v1/auth/personal-access-tokens/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    );
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
  localStorage.setItem(authKeys().access, token.access_token);
  localStorage.setItem(authKeys().refresh, token.refresh_token);
  window.dispatchEvent(new Event(authKeys().event));
}

export function clearAuthToken() {
  localStorage.removeItem(authKeys().access);
  localStorage.removeItem(authKeys().refresh);
  window.dispatchEvent(new Event(authKeys().event));
}

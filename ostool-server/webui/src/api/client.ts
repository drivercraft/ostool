import { authSurface, clearBrowserSession } from "@/auth-session";
export { clearBrowserSession, notifyAuthChanged } from "@/auth-session";
import type { UserInput, RoleInput } from "@/types/api";
import type {
  AdminBoardUpsertRequest,
  AdminServerConfigResponse,
  AdminTftpConfigResponse,
  AdminTftpStatusResponse,
  BoardConfig,
  DtbFileResponse,
  ErrorResponse,
  LoaderImage,
  LoaderUpdateJob,
  TftpConfig,
  UpdateServerConfigRequest,
  VirtualDeviceSummary,
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
export class ApiRequestError extends Error {
  constructor(
    message: string,
    public code: string,
    public status: number,
    public details?: Record<string, unknown>,
  ) {
    super(message);
  }
}
let refreshing: Promise<void> | undefined;
export async function refreshAuth() {
  const refresh = async () => {
    const response = await fetch("/api/v1/auth/refresh", {
      method: "POST",
      credentials: "same-origin",
      headers: { "x-ostool-surface": authSurface() },
    });
    if (!response.ok) {
      clearBrowserSession();
      throw new ApiRequestError("登录已过期，请重新登录", "unauthorized", 401);
    }
  };
  if (!refreshing)
    refreshing = (
      navigator.locks
        ? navigator.locks.request<void>(
            `ostool:${authSurface()}:refresh`,
            async () => {
              await refresh();
            },
          )
        : refresh()
    ).finally(() => {
      refreshing = undefined;
    });
  return refreshing;
}
let stepUp: Promise<void> | undefined;
function verifyIdentity() {
  if (!stepUp)
    stepUp = new Promise<void>((resolve, reject) =>
      window.dispatchEvent(
        new CustomEvent("ostool:step-up", { detail: { resolve, reject } }),
      ),
    ).finally(() => {
      stepUp = undefined;
    });
  return stepUp;
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

  headers.set("x-ostool-surface", authSurface());
  const send = () =>
    fetch(path, {
      ...options,
      credentials: "same-origin",
      headers,
      body,
    }).catch(() => {
      throw new Error(NETWORK_ERROR_MESSAGE);
    });
  let response = await send();
  const authentication =
    path.startsWith("/api/v1/auth/") &&
    ![
      "/api/v1/auth/me",
      "/api/v1/auth/device/authorize",
      "/api/v1/auth/personal-access-tokens",
    ].includes(path) &&
    !path.startsWith("/api/v1/auth/security");
  if (
    isPublicEdition() &&
    response.status === 401 &&
    !authentication &&
    path !== "/api/v1/admin/auth/login"
  ) {
    await refreshAuth();
    response = await send();
  }
  if (!response.ok) {
    const error =
      (await readJsonBody<ErrorResponse>(response).catch(() => null)) ?? null;
    if (
      response.status === 403 &&
      error?.code === "reauth_required" &&
      path !== "/api/v1/auth/step-up"
    ) {
      await verifyIdentity();
      response = await send();
      if (response.ok) return response;
    }
    const failure = response.bodyUsed
      ? error
      : ((await readJsonBody<ErrorResponse>(response).catch(() => null)) ??
        error);
    throw new ApiRequestError(
      failure?.message || `请求失败：${response.status}`,
      failure?.code || "request_failed",
      response.status,
      failure?.details as Record<string, unknown> | undefined,
    );
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
  site?: import("@/types/api").SiteView;
  edition: "local" | "public";
  enabled: boolean;
  allow_registration: boolean;
  database_available: boolean;
  captcha_required: boolean;
  registration_email_verification: boolean;
  email_security_available: boolean;
  password_reset_available?: boolean;
};
let authConfigRequest: Promise<AuthConfig> | undefined;
export const api = {
  legalVersion(kind: "terms" | "privacy", version: number) {
    return request<import("@/types/api").LegalDocument>(
      `/api/v1/site/legal/${kind}/${version}`,
    );
  },
  updateAccountSettings(
    revision: number,
    settings: import("@/types/api").AccountSettings,
  ) {
    return request<void>("/api/v1/admin/account-settings", {
      method: "PUT",
      bodyJson: { revision, settings },
    });
  },
  updateSite(revision: number, settings: import("@/types/api").SiteSettings) {
    return request<void>("/api/v1/admin/site-settings", {
      method: "PUT",
      bodyJson: { revision, settings },
    });
  },
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
    try {
      await fetch("/api/v1/auth/logout", {
        method: "POST",
        credentials: "same-origin",
        headers: { "x-ostool-surface": authSurface() },
      });
    } finally {
      clearBrowserSession();
    }
  },
  adminLogin(email: string, password: string, verification: HumanProof) {
    return request<LoginResult>("/api/v1/admin/auth/login", {
      method: "POST",
      bodyJson: { email, password, verification },
    });
  },
  login(email: string, password: string, verification: HumanProof) {
    return request<LoginResult>("/api/v1/auth/login", {
      method: "POST",
      bodyJson: { email, password, verification },
    });
  },
  register(
    email: string,
    password: string,
    displayName: string,
    verification: HumanProof,
    emailChallenge?: { id: string; code: string },
    legal?: import("@/types/api").LegalConsent,
  ) {
    return request<LoginResult>("/api/v1/auth/register", {
      method: "POST",
      bodyJson: {
        email,
        password,
        display_name: displayName || null,
        legal,
        verification,
        email_challenge_id: emailChallenge?.id,
        email_code: emailChallenge?.code,
      },
    });
  },
  captcha(action: HumanAction) {
    return request<HumanChallenge>(`/api/v1/auth/captcha?action=${action}`);
  },
  verifyMfa(challenge_id: string, code: string) {
    return request<LoginResult>("/api/v1/auth/mfa/verify", {
      method: "POST",
      bodyJson: { challenge_id, code },
    });
  },
  stepUp(password: string, code: string) {
    return request<void>("/api/v1/auth/step-up", {
      method: "POST",
      bodyJson: { password, code },
    });
  },
  securityStatus() {
    return request<SecurityStatus>("/api/v1/auth/security");
  },
  enrollMfa() {
    return request<MfaEnrollment>("/api/v1/auth/security/enrollment", {
      method: "POST",
    });
  },
  confirmMfa(challenge_id: string, code: string) {
    return request<{ recovery_codes: string[] }>(
      "/api/v1/auth/security/confirm",
      { method: "POST", bodyJson: { challenge_id, code } },
    );
  },
  disableMfa() {
    return request<void>("/api/v1/auth/security/mfa", { method: "DELETE" });
  },
  regenerateRecoveryCodes() {
    return request<{ recovery_codes: string[] }>(
      "/api/v1/auth/security/recovery-codes",
      { method: "POST" },
    );
  },
  revokeSession(id: string) {
    return request<{ current: boolean }>(
      `/api/v1/auth/security/sessions/${id}`,
      { method: "DELETE" },
    );
  },
  revokeOtherSessions() {
    return request<void>("/api/v1/auth/security/sessions/others", {
      method: "POST",
    });
  },
  emailCode(
    email: string,
    action: "register" | "password_reset",
    verification: HumanProof,
  ) {
    return request<EmailChallenge>("/api/v1/auth/email-code", {
      method: "POST",
      bodyJson: { email, action, verification },
    });
  },
  resetPassword(challenge_id: string, code: string, new_password: string) {
    return request<void>("/api/v1/auth/password-reset", {
      method: "POST",
      bodyJson: { challenge_id, code, new_password },
    });
  },
  verifyEmailSend() {
    return request<EmailChallenge>("/api/v1/auth/security/email", {
      method: "POST",
    });
  },
  verifyEmailConfirm(challenge_id: string, code: string) {
    return request<void>("/api/v1/auth/security/email/confirm", {
      method: "POST",
      bodyJson: { challenge_id, code },
    });
  },
  authorizeDevice(user_code: string) {
    return request<void>("/api/v1/auth/device/authorize", {
      method: "POST",
      bodyJson: { user_code },
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
  deleteSession(sessionId: string) {
    return request<void>(
      `/api/v1/admin/sessions/${encodeURIComponent(sessionId)}`,
      {
        method: "DELETE",
      },
    );
  },
  updateTftpConfig(tftp: TftpConfig) {
    return request<AdminTftpConfigResponse>("/api/v1/admin/tftp", {
      method: "PUT",
      bodyJson: tftp,
    });
  },
  reconcileTftp() {
    return request<AdminTftpStatusResponse>("/api/v1/admin/tftp/reconcile", {
      method: "POST",
    });
  },
  updateServerConfig(payload: UpdateServerConfigRequest) {
    return request<AdminServerConfigResponse>("/api/v1/admin/server-config", {
      method: "PUT",
      bodyJson: payload,
    });
  },
};

export type HumanAction =
  | "login"
  | "admin_login"
  | "register"
  | "registration_email"
  | "password_reset";
export interface HumanProof {
  captcha_id?: string;
  captcha_code?: string;
}
export interface HumanChallenge {
  id: string;
  action: HumanAction;
  provider: "image" | "turnstile";
  image_data_url?: string;
  site_key?: string;
  expires_in: number;
}
export type LoginResult =
  | { authenticated: true; enrollment_required?: boolean; mfa_required?: false }
  | { mfa_required: true; challenge_id: string; expires_in: number };
export interface SecurityStatus {
  enabled: boolean;
  available: boolean;
  recovery_codes_remaining: number;
  email_verified: boolean;
  email_verification_available: boolean;
  require_admin_mfa: boolean;
}
export interface MfaEnrollment {
  challenge_id: string;
  secret: string;
  otpauth_uri: string;
  qr_image: string;
  expires_in: number;
}
export interface SecuritySession {
  id: string;
  client: string;
  created_at: string;
  last_seen_at: string;
  expires_at: string;
  ip: string | null;
  user_agent: string | null;
  current: boolean;
}
export interface EmailChallenge {
  challenge_id: string;
  expires_in: number;
  resend_after: number;
  message: string;
}

export interface AuthUser {
  id: string;
  email: string;
  display_name: string | null;
  is_admin: boolean;
  mfa_enrollment_required?: boolean;
}

export interface PersonalAccessToken {
  id: string;
  token: string;
  name: string;
  expires_at: string | null;
}

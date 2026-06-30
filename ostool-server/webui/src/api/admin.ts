import type {
  AdminBoardUpsertRequest,
  AdminLeaseCreateRequest,
  AdminLeaseUpdateRequest,
  AdminOverviewResponse,
  AdminPasswordResetRequest,
  AdminPermissionsResponse,
  AdminRoleCreateRequest,
  AdminRoleDisableRequest,
  AdminRoleResponse,
  AdminRolesResponse,
  AdminRoleUpdateRequest,
  AdminServerConfigResponse,
  AdminSessionsResponse,
  AdminSessionUpdateRequest,
  AdminSessionResponse,
  SiteSettingsResponse,
  SiteSettingsUpdateRequest,
  AdminTftpConfigResponse,
  AdminTftpStatusResponse,
  AdminUserCreateRequest,
  AdminUserResponse,
  AdminUserRolesResponse,
  AdminUserRolesUpdateRequest,
  AdminUsersResponse,
  AdminUserUpdateRequest,
  BoardConfig,
  DtbMetadataInput,
  DtbFileResponse,
  LeaseResponse,
  LeasesResponse,
  NetworkInterfaceSummary,
  SerialPortSummary,
  TftpConfig,
  UpdateServerConfigRequest,
} from "@/types/api";

import { request } from "./http";

function dtbHeaders(dtbName?: string | null, metadata?: DtbMetadataInput) {
  const headers = new Headers();
  if (dtbName) {
    headers.set("X-Dtb-Name", dtbName);
  }
  if (metadata?.boot_architecture) {
    headers.set("X-Dtb-Architecture", metadata.boot_architecture);
  }
  if (metadata?.compatible) {
    headers.set("X-Dtb-Compatible", metadata.compatible);
  }
  if (metadata?.description) {
    headers.set("X-Dtb-Description", metadata.description);
  }
  if (typeof metadata?.disabled === "boolean") {
    headers.set("X-Dtb-Disabled", metadata.disabled ? "true" : "false");
  }
  return headers;
}

export const adminApi = {
  getOverview() {
    return request<AdminOverviewResponse>("/api/v1/admin/overview");
  },
  listBoards() {
    return request<BoardConfig[]>("/api/v1/admin/boards");
  },
  getBoard(boardId: string) {
    return request<BoardConfig>(`/api/v1/admin/boards/${encodeURIComponent(boardId)}`);
  },
  createBoard(payload: AdminBoardUpsertRequest) {
    return request<BoardConfig>("/api/v1/admin/boards", {
      method: "POST",
      bodyJson: payload,
    });
  },
  updateBoard(boardId: string, payload: AdminBoardUpsertRequest) {
    return request<BoardConfig>(`/api/v1/admin/boards/${encodeURIComponent(boardId)}`, {
      method: "PUT",
      bodyJson: payload,
    });
  },
  deleteBoard(boardId: string) {
    return request<void>(`/api/v1/admin/boards/${encodeURIComponent(boardId)}`, {
      method: "DELETE",
    });
  },
  listDtbs() {
    return request<DtbFileResponse[]>("/api/v1/admin/dtbs");
  },
  getDtb(dtbName: string) {
    return request<DtbFileResponse>(`/api/v1/admin/dtbs/${encodeURIComponent(dtbName)}`);
  },
  createDtb(dtbName: string, file: Blob, metadata?: DtbMetadataInput) {
    return request<DtbFileResponse>("/api/v1/admin/dtbs", {
      method: "POST",
      headers: dtbHeaders(dtbName, metadata),
      body: file,
    });
  },
  updateDtb(
    currentName: string,
    nextName?: string | null,
    file?: Blob | null,
    metadata?: DtbMetadataInput,
  ) {
    return request<DtbFileResponse>(`/api/v1/admin/dtbs/${encodeURIComponent(currentName)}`, {
      method: "PUT",
      headers: dtbHeaders(nextName, metadata),
      body: file ?? undefined,
    });
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
    return request<NetworkInterfaceSummary[]>("/api/v1/admin/network-interfaces");
  },
  listSessions() {
    return request<AdminSessionsResponse>("/api/v1/admin/sessions");
  },
  deleteSession(sessionId: string) {
    return request<void>(`/api/v1/admin/sessions/${encodeURIComponent(sessionId)}`, {
      method: "DELETE",
    });
  },
  updateSession(sessionId: string, payload: AdminSessionUpdateRequest) {
    return request<AdminSessionResponse>(`/api/v1/admin/sessions/${encodeURIComponent(sessionId)}`, {
      method: "PUT",
      bodyJson: payload,
    });
  },
  closeSession(sessionId: string) {
    return request<void>(`/api/v1/admin/sessions/${encodeURIComponent(sessionId)}/close`, {
      method: "POST",
    });
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
  getSiteSettings() {
    return request<SiteSettingsResponse>("/api/v1/admin/site-settings");
  },
  updateSiteSettings(payload: SiteSettingsUpdateRequest) {
    return request<SiteSettingsResponse>("/api/v1/admin/site-settings", {
      method: "PUT",
      bodyJson: payload,
    });
  },
  listAdminUsers() {
    return request<AdminUsersResponse>("/api/v1/admin/users");
  },
  createAdminUser(payload: AdminUserCreateRequest) {
    return request<AdminUserResponse>("/api/v1/admin/users", {
      method: "POST",
      bodyJson: payload,
    });
  },
  getAdminUser(userId: string) {
    return request<AdminUserResponse>(`/api/v1/admin/users/${encodeURIComponent(userId)}`);
  },
  updateAdminUser(userId: string, payload: AdminUserUpdateRequest) {
    return request<AdminUserResponse>(`/api/v1/admin/users/${encodeURIComponent(userId)}`, {
      method: "PUT",
      bodyJson: payload,
    });
  },
  deleteAdminUser(userId: string) {
    return request<void>(`/api/v1/admin/users/${encodeURIComponent(userId)}`, {
      method: "DELETE",
    });
  },
  resetAdminUserPassword(userId: string, payload: AdminPasswordResetRequest) {
    return request<void>(`/api/v1/admin/users/${encodeURIComponent(userId)}/reset-password`, {
      method: "POST",
      bodyJson: payload,
    });
  },
  disableAdminUser(userId: string) {
    return request<void>(`/api/v1/admin/users/${encodeURIComponent(userId)}/disable`, {
      method: "POST",
    });
  },
  getAdminUserRoles(userId: string) {
    return request<AdminUserRolesResponse>(
      `/api/v1/admin/users/${encodeURIComponent(userId)}/roles`,
    );
  },
  updateAdminUserRoles(userId: string, payload: AdminUserRolesUpdateRequest) {
    return request<AdminUserRolesResponse>(
      `/api/v1/admin/users/${encodeURIComponent(userId)}/roles`,
      {
        method: "PUT",
        bodyJson: payload,
      },
    );
  },
  listAdminPermissions() {
    return request<AdminPermissionsResponse>("/api/v1/admin/permissions");
  },
  listAdminRoles() {
    return request<AdminRolesResponse>("/api/v1/admin/roles");
  },
  getAdminRole(roleId: string) {
    return request<AdminRoleResponse>(`/api/v1/admin/roles/${encodeURIComponent(roleId)}`);
  },
  createAdminRole(payload: AdminRoleCreateRequest) {
    return request<AdminRoleResponse>("/api/v1/admin/roles", {
      method: "POST",
      bodyJson: payload,
    });
  },
  updateAdminRole(roleId: string, payload: AdminRoleUpdateRequest) {
    return request<AdminRoleResponse>(`/api/v1/admin/roles/${encodeURIComponent(roleId)}`, {
      method: "PUT",
      bodyJson: payload,
    });
  },
  disableAdminRole(roleId: string, payload: AdminRoleDisableRequest) {
    return request<AdminRoleResponse>(`/api/v1/admin/roles/${encodeURIComponent(roleId)}/disable`, {
      method: "POST",
      bodyJson: payload,
    });
  },
  deleteAdminRole(roleId: string) {
    return request<void>(`/api/v1/admin/roles/${encodeURIComponent(roleId)}`, {
      method: "DELETE",
    });
  },
  listAdminLeases() {
    return request<LeasesResponse>("/api/v1/admin/leases");
  },
  createAdminLease(payload: AdminLeaseCreateRequest) {
    return request<LeaseResponse>("/api/v1/admin/leases", {
      method: "POST",
      bodyJson: payload,
    });
  },
  getAdminLease(leaseId: string) {
    return request<LeaseResponse>(
      `/api/v1/admin/leases/${encodeURIComponent(leaseId)}`,
    );
  },
  updateAdminLease(leaseId: string, payload: AdminLeaseUpdateRequest) {
    return request<LeaseResponse>(
      `/api/v1/admin/leases/${encodeURIComponent(leaseId)}`,
      {
        method: "PUT",
        bodyJson: payload,
      },
    );
  },
  startAdminLeaseSession(leaseId: string) {
    return request<LeaseResponse>(
      `/api/v1/admin/leases/${encodeURIComponent(leaseId)}/session`,
      {
        method: "POST",
      },
    );
  },
  releaseAdminLease(leaseId: string) {
    return request<void>(
      `/api/v1/admin/leases/${encodeURIComponent(leaseId)}/release`,
      {
        method: "POST",
      },
    );
  },
  deleteAdminLease(leaseId: string) {
    return request<void>(`/api/v1/admin/leases/${encodeURIComponent(leaseId)}`, {
      method: "DELETE",
    });
  },
};

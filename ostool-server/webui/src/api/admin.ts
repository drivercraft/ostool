import type {
  AdminBoardUpsertRequest,
  AdminOverviewResponse,
  AdminPasswordResetRequest,
  AdminPermissionsResponse,
  AdminRoleCreateRequest,
  AdminRoleResponse,
  AdminRolesResponse,
  AdminRoleUpdateRequest,
  AdminServerConfigResponse,
  AdminSessionsResponse,
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
  DtbFileResponse,
  LeasesResponse,
  NetworkInterfaceSummary,
  SerialPortSummary,
  TftpConfig,
  UpdateServerConfigRequest,
} from "@/types/api";

import { request } from "./http";

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
    return request<DtbFileResponse>(`/api/v1/admin/dtbs/${encodeURIComponent(currentName)}`, {
      method: "PUT",
      headers,
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
  deleteAdminRole(roleId: string) {
    return request<void>(`/api/v1/admin/roles/${encodeURIComponent(roleId)}`, {
      method: "DELETE",
    });
  },
  listAdminLeases() {
    return request<LeasesResponse>("/api/v1/admin/leases");
  },
  deleteAdminLease(leaseId: string) {
    return request<void>(`/api/v1/admin/leases/${encodeURIComponent(leaseId)}`, {
      method: "DELETE",
    });
  },
};

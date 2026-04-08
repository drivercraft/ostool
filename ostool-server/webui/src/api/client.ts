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
  NetworkInterfaceSummary,
  SerialPortSummary,
  TftpConfig,
  UpdateServerConfigRequest,
} from "@/types/api";

type RequestOptions = RequestInit & {
  bodyJson?: unknown;
};

async function readJsonBody<T>(response: Response): Promise<T | undefined> {
  const text = await response.text();
  if (!text.trim()) {
    return undefined;
  }
  return JSON.parse(text) as T;
}

async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const headers = new Headers(options.headers);
  let body = options.body;

  if (options.bodyJson !== undefined) {
    headers.set("content-type", "application/json");
    body = JSON.stringify(options.bodyJson);
  }

  const response = await fetch(path, {
    ...options,
    headers,
    body,
  });

  if (!response.ok) {
    const error = (await readJsonBody<ErrorResponse>(response).catch(() => null)) ?? null;
    throw new Error(error?.message || `请求失败：${response.status}`);
  }

  return (await readJsonBody<T>(response)) as T;
}

export const api = {
  getOverview() {
    return request<AdminOverviewResponse>("/api/v1/admin/overview");
  },
  listBoards() {
    return request<BoardConfig[]>("/api/v1/admin/boards");
  },
  getBoard(boardId: string) {
    return request<BoardConfig>(`/api/v1/admin/boards/${encodeURIComponent(boardId)}`);
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
};

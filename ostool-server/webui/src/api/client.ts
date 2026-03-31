import type {
  AdminOverviewResponse,
  AdminServerConfigResponse,
  AdminSessionsResponse,
  AdminTftpConfigResponse,
  AdminTftpStatusResponse,
  BoardEditorDocument,
  BoardConfig,
  ErrorResponse,
  NetworkInterfaceSummary,
  TftpConfig,
  UpdateServerConfigRequest,
} from "@/types/api";

type RequestOptions = RequestInit & {
  bodyJson?: unknown;
};

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
    const error = (await response.json().catch(() => null)) as ErrorResponse | null;
    throw new Error(error?.message || `请求失败：${response.status}`);
  }

  if (response.status === 204) {
    return undefined as T;
  }

  return (await response.json()) as T;
}

export const api = {
  getOverview() {
    return request<AdminOverviewResponse>("/api/v1/admin/overview");
  },
  listBoards() {
    return request<BoardConfig[]>("/api/v1/admin/boards");
  },
  listNetworkInterfaces() {
    return request<NetworkInterfaceSummary[]>("/api/v1/admin/network-interfaces");
  },
  getNewBoardEditor() {
    return request<BoardEditorDocument>("/api/v1/admin/boards/editor");
  },
  getBoardEditor(boardId: string) {
    return request<BoardEditorDocument>(`/api/v1/admin/boards/${encodeURIComponent(boardId)}`);
  },
  createBoard(document: BoardEditorDocument) {
    return request<BoardEditorDocument>("/api/v1/admin/boards", {
      method: "POST",
      bodyJson: document,
    });
  },
  updateBoard(boardId: string, document: BoardEditorDocument) {
    return request<BoardEditorDocument>(`/api/v1/admin/boards/${encodeURIComponent(boardId)}`, {
      method: "PUT",
      bodyJson: document,
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

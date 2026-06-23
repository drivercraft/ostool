import type {
  CreateSessionRequest,
  SessionCreatedResponse,
  SessionDetailResponse,
} from "@/types/api";

import { request } from "./http";

export const sessionsApi = {
  createSession(payload: CreateSessionRequest) {
    return request<SessionCreatedResponse>("/api/v1/sessions", {
      method: "POST",
      bodyJson: payload,
    });
  },
  getSession(sessionId: string) {
    return request<SessionDetailResponse>(
      `/api/v1/sessions/${encodeURIComponent(sessionId)}`,
    );
  },
  releaseSession(sessionId: string) {
    return request<void>(`/api/v1/sessions/${encodeURIComponent(sessionId)}`, {
      method: "DELETE",
    });
  },
};

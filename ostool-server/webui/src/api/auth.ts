import type { CurrentUserResponse, LoginRequest } from "@/types/api";

import { request } from "./http";

export const authApi = {
  login(payload: LoginRequest) {
    return request<CurrentUserResponse>("/api/v1/auth/login", {
      method: "POST",
      bodyJson: payload,
    });
  },
  logout() {
    return request<void>("/api/v1/auth/logout", {
      method: "POST",
    });
  },
  getCurrentUser() {
    return request<CurrentUserResponse>("/api/v1/auth/me");
  },
  getUserProfile() {
    return request<CurrentUserResponse>("/api/v1/user/profile");
  },
};

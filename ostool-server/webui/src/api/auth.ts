import type {
  CaptchaResponse,
  CurrentUserResponse,
  LoginRequest,
  RegisterRequest,
  RegisterResponse,
  RegistrationPolicyResponse,
} from "@/types/api";

import { request } from "./http";

export const authApi = {
  getCaptcha() {
    return request<CaptchaResponse>("/api/v1/auth/captcha");
  },
  getRegistrationPolicy() {
    return request<RegistrationPolicyResponse>("/api/v1/auth/registration-policy");
  },
  register(payload: RegisterRequest) {
    return request<RegisterResponse>("/api/v1/auth/register", {
      method: "POST",
      bodyJson: payload,
    });
  },
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

import { adminApi } from "./admin";
import { authApi } from "./auth";
import { publicApi } from "./public";
import { sessionsApi } from "./sessions";
import { userApi } from "./user";

export const api = {
  auth: authApi,
  public: publicApi,
  user: userApi,
  sessions: sessionsApi,
  admin: adminApi,
};

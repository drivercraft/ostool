import { adminApi } from "./admin";
import { authApi } from "./auth";
import { publicApi } from "./public";
import { sessionsApi } from "./sessions";
import { userApi } from "./user";

export const api = {
  ...authApi,
  ...publicApi,
  ...userApi,
  ...sessionsApi,
  ...adminApi,
};

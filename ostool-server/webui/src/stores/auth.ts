import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { api } from "@/api";
import type { CurrentUserResponse } from "@/types/api";

export type AuthUser = CurrentUserResponse;

const adminPermissionModules = new Set([
  "overview",
  "users",
  "roles",
  "boards",
  "dtbs",
  "leases",
  "sessions",
  "tftp",
  "server",
  "site",
  "serial_ports",
  "network_interfaces",
  "permissions",
]);

export const useAuthStore = defineStore("auth", () => {
  const user = ref<AuthUser | null>(null);
  const loaded = ref(false);

  const isAuthenticated = computed(() => user.value !== null);
  const isAdmin = computed(
    () =>
      user.value?.roles.some((role) => role.name === "admin") ||
      user.value?.permissions.some((permission) => {
        const [moduleName] = permission.code.split(".");
        return adminPermissionModules.has(moduleName);
      }) ||
      false,
  );
  const currentUser = computed(() => user.value);
  const admin = computed(() => (isAdmin.value ? user.value : null));

  function hasPermission(permissionCode: string) {
    if (user.value?.roles.some((role) => role.name === "admin")) {
      return true;
    }
    return Boolean(user.value?.permissions.some((permission) => permission.code === permissionCode));
  }

  async function loadCurrentUser() {
    try {
      user.value = await api.getCurrentUser();
    } catch {
      user.value = null;
    } finally {
      loaded.value = true;
    }
  }

  async function login(username: string, password: string) {
    user.value = await api.login({ username, password });
    loaded.value = true;
  }

  async function logoutUser() {
    await api.logout().catch(() => undefined);
    user.value = null;
    loaded.value = true;
  }

  async function logoutAdmin() {
    await logoutUser();
  }

  async function logoutAll() {
    await logoutUser();
  }

  return {
    user,
    admin,
    loaded,
    isAuthenticated,
    isAdmin,
    currentUser,
    hasPermission,
    loadCurrentUser,
    login,
    logoutUser,
    logoutAdmin,
    logoutAll,
  };
});

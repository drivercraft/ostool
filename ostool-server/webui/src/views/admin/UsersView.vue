<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";

import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { USERNAME_PATTERN, VALIDATION_LIMITS } from "@/constants/validation";
import { useUiStore } from "@/stores/ui";
import type {
  AdminRoleResponse,
  AdminUserResponse,
} from "@/types/api";

type ModalMode = "create" | "edit" | "reset-password" | null;

const ui = useUiStore();
const users = ref<AdminUserResponse[]>([]);
const roles = ref<AdminRoleResponse[]>([]);
const userRoleIds = ref<Record<string, string[]>>({});
const roleNamesById = computed(() =>
  new Map(roles.value.map((role) => [role.id, role.display_name])),
);
const disabledRoleIds = computed(() =>
  new Set(roles.value.filter((role) => role.disabled).map((role) => role.id)),
);

const loading = ref(true);
const submitting = ref(false);

const search = ref("");
const statusFilter = ref<"all" | "active" | "disabled">("all");
const roleFilter = ref<string>("");

const filteredUsers = computed(() =>
  users.value.filter((user) => {
    if (search.value) {
      const q = search.value.toLowerCase();
      const haystack =
        `${user.username} ${user.display_name} ${user.email}`.toLowerCase();
      if (!haystack.includes(q)) {
        return false;
      }
    }
    const unavailable = userUnavailable(user);
    if (statusFilter.value === "active" && unavailable) {
      return false;
    }
    if (statusFilter.value === "disabled" && !unavailable) {
      return false;
    }
    if (roleFilter.value) {
      const ids = userRoleIds.value[user.id] ?? [];
      if (!ids.includes(roleFilter.value)) {
        return false;
      }
    }
    return true;
  }),
);

/* ---- 更多操作下拉菜单 ---- */
const openMenuUserId = ref<string | null>(null);
const menuPosition = ref({ top: 0, left: 0 });
function toggleMenu(userId: string, event: MouseEvent) {
  if (openMenuUserId.value === userId) {
    closeMenu();
    return;
  }
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  menuPosition.value = {
    top: rect.bottom + 6,
    left: Math.max(12, rect.right - 180),
  };
  openMenuUserId.value = userId;
}
function closeMenu() {
  openMenuUserId.value = null;
}
function onDocumentClick(event: MouseEvent) {
  const target = event.target as HTMLElement | null;
  if (target && !target.closest(".row-action-menu") && !target.closest(".action-menu")) {
    closeMenu();
  }
}
onMounted(() => document.addEventListener("click", onDocumentClick));
onUnmounted(() => document.removeEventListener("click", onDocumentClick));

/* ---- 弹窗 ---- */
const modalMode = ref<ModalMode>(null);
const modalUser = ref<AdminUserResponse | null>(null);
const pointerDownOnModalOverlay = ref(false);
const form = ref({
  username: "",
  display_name: "",
  email: "",
  password: "",
  role_ids: [] as string[],
  disabled: false,
});

function openCreate() {
  modalMode.value = "create";
  modalUser.value = null;
  form.value = {
    username: "",
    display_name: "",
    email: "",
    password: "",
    role_ids: [],
    disabled: false,
  };
}

function userHasDisabledRole(user: AdminUserResponse) {
  return (userRoleIds.value[user.id] ?? []).some((roleId) => disabledRoleIds.value.has(roleId));
}

function userUnavailable(user: AdminUserResponse) {
  return user.disabled || userHasDisabledRole(user);
}

function userStatus(user: AdminUserResponse) {
  if (user.disabled) {
    return { tone: "neutral" as const, label: "已禁用" };
  }
  if (userHasDisabledRole(user)) {
    return { tone: "neutral" as const, label: "角色已禁用" };
  }
  return { tone: "good" as const, label: "启用" };
}

function openEdit(user: AdminUserResponse) {
  modalMode.value = "edit";
  modalUser.value = user;
  form.value = {
    username: user.username,
    display_name: user.display_name,
    email: user.email,
    password: "",
    role_ids: [...(userRoleIds.value[user.id] ?? [])],
    disabled: user.disabled,
  };
  closeMenu();
}

function openResetPassword(user: AdminUserResponse) {
  modalMode.value = "reset-password";
  modalUser.value = user;
  form.value.password = "";
  closeMenu();
}

function closeModal() {
  modalMode.value = null;
  modalUser.value = null;
}

function onModalOverlayPointerDown(event: PointerEvent) {
  pointerDownOnModalOverlay.value = event.target === event.currentTarget;
}

function onModalOverlayClick(event: MouseEvent) {
  if (pointerDownOnModalOverlay.value && event.target === event.currentTarget) {
    closeModal();
  }
  pointerDownOnModalOverlay.value = false;
}

/* ---- 数据加载 ---- */
async function loadUsers() {
  loading.value = true;
  try {
    const [userResponse, roleResponse] = await Promise.all([
      api.listAdminUsers(),
      api.listAdminRoles(),
    ]);
    users.value = userResponse.users;
    roles.value = roleResponse.roles;
    const rolePairs = await Promise.all(
      userResponse.users.map(async (user) => [
        user.id,
        (await api.getAdminUserRoles(user.id)).roles.map((role) => role.id),
      ] as const),
    );
    userRoleIds.value = Object.fromEntries(rolePairs);
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

/* ---- 提交 ---- */
async function submitCreate() {
  if (!form.value.username.trim() || !form.value.display_name.trim() || !form.value.email.trim() || !form.value.password) {
    ui.setError("请填写用户名、显示名、邮箱和密码");
    return;
  }
  if (form.value.password.length < VALIDATION_LIMITS.passwordMin) {
    ui.setError(`密码至少需要 ${VALIDATION_LIMITS.passwordMin} 位`);
    return;
  }
  submitting.value = true;
  try {
    const created = await api.createAdminUser({
      username: form.value.username.trim(),
      display_name: form.value.display_name.trim() || form.value.username.trim(),
      email: form.value.email.trim(),
      password: form.value.password,
      role_ids: form.value.role_ids,
    });
    if (form.value.role_ids.length > 0) {
      await api.updateAdminUserRoles(created.id, {
        role_ids: form.value.role_ids,
      });
    }
    ui.setSuccess(`已创建用户 ${created.username}`);
    closeModal();
    await loadUsers();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    submitting.value = false;
  }
}

async function submitEdit() {
  if (!modalUser.value) {
    return;
  }
  if (!form.value.display_name.trim() || !form.value.email.trim()) {
    ui.setError("请填写显示名和邮箱");
    return;
  }
  submitting.value = true;
  try {
    const userId = modalUser.value.id;
    await api.updateAdminUser(userId, {
      display_name: form.value.display_name.trim() || modalUser.value.username,
      email: form.value.email.trim(),
      disabled: form.value.disabled,
    });
    await api.updateAdminUserRoles(userId, {
      role_ids: form.value.role_ids,
    });
    ui.setSuccess(`已更新用户 ${modalUser.value.username}`);
    closeModal();
    await loadUsers();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    submitting.value = false;
  }
}

async function submitResetPassword() {
  if (!modalUser.value) {
    return;
  }
  if (form.value.password.length < VALIDATION_LIMITS.passwordMin) {
    ui.setError(`新密码至少需要 ${VALIDATION_LIMITS.passwordMin} 位`);
    return;
  }
  submitting.value = true;
  try {
    await api.resetAdminUserPassword(modalUser.value.id, {
      password: form.value.password,
    });
    ui.setSuccess(`已重置 ${modalUser.value.username} 的密码`);
    closeModal();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    submitting.value = false;
  }
}

async function toggleDisabled(user: AdminUserResponse) {
  const action = user.disabled ? "启用" : "禁用";
  const confirmed = await ui.confirm({
    tone: user.disabled ? "info" : "danger",
    title: `${action}用户`,
    message: `确认${action}用户 ${user.username}？`,
    confirmLabel: action,
  });
  if (!confirmed) {
    return;
  }
  try {
    if (user.disabled) {
      await api.updateAdminUser(user.id, {
        display_name: user.display_name,
        email: user.email,
        disabled: false,
      });
    } else {
      await api.disableAdminUser(user.id);
    }
    ui.setSuccess(`已${action}用户 ${user.username}`);
    await loadUsers();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

async function deleteUser(user: AdminUserResponse) {
  if (user.disabled) {
    closeMenu();
    return;
  }
  const confirmed = await ui.confirm({
    tone: "danger",
    title: "删除用户",
    message: `确认删除用户 ${user.username}？`,
    confirmLabel: "删除",
  });
  if (!confirmed) {
    return;
  }
  try {
    await api.deleteAdminUser(user.id);
    ui.setSuccess(`已删除用户 ${user.username}`);
    closeMenu();
    await loadUsers();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

function submitModal() {
  if (modalMode.value === "create") {
    void submitCreate();
  } else if (modalMode.value === "edit") {
    void submitEdit();
  } else if (modalMode.value === "reset-password") {
    void submitResetPassword();
  }
}

function roleLabels(user: AdminUserResponse): string[] {
  return (userRoleIds.value[user.id] ?? [])
    .map((id) => roleNamesById.value.get(id))
    .filter((v): v is string => Boolean(v));
}

onMounted(() => {
  ui.clearMessages();
  void loadUsers();
});
</script>

<template>
  <section class="page-grid users-page admin-list-page">
    <div class="panel admin-table-panel">
      <div class="admin-toolbar">
        <div class="admin-toolbar-left">
          <button class="btn btn-primary" @click="openCreate">
            <Icon name="plus" :size="16" class="btn-icon" />
            新增用户
          </button>
        </div>
        <div class="admin-toolbar-right">
          <label class="search-field">
            <Icon name="search" :size="16" />
            <input
              v-model="search"
              type="search"
              maxlength="128"
              placeholder="按用户名 / 显示名 / 邮箱搜索"
            />
          </label>
          <label class="field filter-field">
            <span>状态</span>
            <select v-model="statusFilter">
              <option value="all">全部状态</option>
              <option value="active">启用</option>
              <option value="disabled">已禁用</option>
            </select>
          </label>
          <label class="field filter-field">
            <span>角色</span>
            <select v-model="roleFilter">
              <option value="">全部角色</option>
              <option v-for="role in roles" :key="role.id" :value="role.id">
                {{ role.display_name }}
              </option>
            </select>
          </label>
        </div>
      </div>

      <div v-if="loading" class="empty-state">正在加载用户...</div>
      <div v-else class="table-scroll">
        <table class="data-table">
          <thead>
            <tr>
              <th class="col-index">序号</th>
              <th>用户</th>
              <th>显示名</th>
              <th>邮箱</th>
              <th>角色</th>
              <th>状态</th>
              <th>创建时间</th>
              <th class="col-actions">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(user, index) in filteredUsers" :key="user.id">
              <td class="col-index">{{ index + 1 }}</td>
              <td>
                <div class="table-cell-stack">
                  <div class="table-cell-stack-body">
                    <span class="table-cell-main">{{ user.username }}</span>
                    <span class="table-cell-sub table-cell-sub--mono">{{ user.id }}</span>
                  </div>
                </div>
              </td>
              <td>{{ user.display_name || "-" }}</td>
              <td>{{ user.email || "-" }}</td>
              <td>
                <div class="role-chip-list">
                  <span
                    v-for="label in roleLabels(user)"
                    :key="label"
                    class="tag-chip"
                  >
                    {{ label }}
                  </span>
                  <span v-if="roleLabels(user).length === 0" class="muted">无</span>
                </div>
              </td>
              <td>
                <StatusPill
                  :tone="userStatus(user).tone"
                  :label="userStatus(user).label"
                />
              </td>
              <td class="muted">{{ new Date(user.created_at).toLocaleString() }}</td>
              <td class="col-actions">
                <div class="row-actions">
                  <button
                    class="btn-icon-only"
                    title="编辑"
                    @click="openEdit(user)"
                  >
                    <Icon name="edit" :size="16" />
                  </button>
                  <button
                    class="btn-icon-only"
                    :title="user.disabled ? '启用' : '禁用'"
                    @click="toggleDisabled(user)"
                  >
                    <Icon :name="user.disabled ? 'check' : 'ban'" :size="16" />
                  </button>
                  <div class="row-action-menu">
                    <button
                      class="btn-icon-only"
                      title="更多"
                      :aria-expanded="openMenuUserId === user.id"
                      @click.stop="toggleMenu(user.id, $event)"
                    >
                      <Icon name="more-vertical" :size="16" />
                    </button>
                  </div>
                  <Teleport to="body">
                    <div
                      v-if="openMenuUserId === user.id"
                      class="action-menu action-menu--floating"
                      :style="{ top: `${menuPosition.top}px`, left: `${menuPosition.left}px` }"
                    >
                      <button
                        class="action-menu-item"
                        @click="openEdit(user)"
                      >
                        <Icon name="edit" :size="14" />
                        编辑用户
                      </button>
                      <button
                        class="action-menu-item"
                        @click="openResetPassword(user)"
                      >
                        <Icon name="key-reset" :size="14" />
                        重置密码
                      </button>
                      <button
                        class="action-menu-item"
                        :disabled="user.disabled"
                        @click="deleteUser(user)"
                      >
                        <Icon name="trash" :size="14" />
                        删除用户
                      </button>
                    </div>
                  </Teleport>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div v-if="!loading" class="table-statusbar">
        <span>{{ filteredUsers.length === 0 ? "暂无分页" : "第 1 / 共 1 页" }}</span>
        <span>本页 {{ filteredUsers.length }} 条</span>
        <span>筛选后 {{ filteredUsers.length }} 条 / 共 {{ users.length }} 条</span>
      </div>
    </div>

    <!-- 创建 / 编辑 / 重置密码 弹窗 -->
    <div
      v-if="modalMode"
      class="modal-overlay"
      @pointerdown="onModalOverlayPointerDown"
      @click="onModalOverlayClick"
    >
      <div
        class="modal-card"
        :class="modalMode === 'reset-password' ? 'modal-card--narrow' : 'modal-card--user-form'"
      >
        <header class="modal-header">
          <h3>
            {{
              modalMode === "create"
                ? "新增用户"
                : modalMode === "edit"
                  ? `编辑 ${modalUser?.username}`
                  : `重置 ${modalUser?.username} 的密码`
            }}
          </h3>
          <button class="btn-icon-only modal-close-button" title="关闭" @click="closeModal">×</button>
        </header>

        <form class="modal-form" @submit.prevent="submitModal">
          <div class="modal-body modal-body-grid">
            <template v-if="modalMode === 'create'">
              <label class="field is-required">
                <span>用户名</span>
                <input
                  v-model="form.username"
                  autocomplete="off"
                  :minlength="VALIDATION_LIMITS.usernameMin"
                  :maxlength="VALIDATION_LIMITS.usernameMax"
                  :pattern="USERNAME_PATTERN"
                  placeholder="登录账号，必须唯一"
                />
              </label>
              <label class="field is-required">
                <span>显示名</span>
                <input
                  v-model="form.display_name"
                  autocomplete="off"
                  :minlength="VALIDATION_LIMITS.displayNameMin"
                  :maxlength="VALIDATION_LIMITS.displayNameMax"
                  placeholder="页面展示名称，例如 张三"
                />
              </label>
              <label class="field is-required">
                <span>邮箱</span>
                <input
                  v-model="form.email"
                  type="email"
                  autocomplete="off"
                  :minlength="VALIDATION_LIMITS.emailMin"
                  :maxlength="VALIDATION_LIMITS.emailMax"
                  placeholder="用户联系邮箱，例如 user@example.com"
                />
              </label>
              <label class="field is-required">
                <span>密码</span>
                <input
                  v-model="form.password"
                  type="password"
                  autocomplete="new-password"
                  :minlength="VALIDATION_LIMITS.passwordMin"
                  :maxlength="VALIDATION_LIMITS.passwordMax"
                  placeholder="初始密码，建议至少 8 位"
                />
              </label>
            </template>

            <template v-else-if="modalMode === 'edit'">
              <label class="field">
                <span>用户名</span>
                <input :value="form.username" disabled />
              </label>
              <label class="field is-required">
                <span>显示名</span>
                <input
                  v-model="form.display_name"
                  :minlength="VALIDATION_LIMITS.displayNameMin"
                  :maxlength="VALIDATION_LIMITS.displayNameMax"
                  placeholder="页面展示名称"
                />
              </label>
              <label class="field is-required">
                <span>邮箱</span>
                <input
                  v-model="form.email"
                  type="email"
                  :minlength="VALIDATION_LIMITS.emailMin"
                  :maxlength="VALIDATION_LIMITS.emailMax"
                  placeholder="用户联系邮箱，例如 user@example.com"
                />
              </label>
              <label class="toggle-field">
                <span class="toggle-switch">
                  <input v-model="form.disabled" type="checkbox" />
                  <span class="toggle-track" />
                  <span class="toggle-knob" />
                </span>
                <span class="toggle-label">
                  {{ form.disabled ? "已禁用登录" : "允许登录" }}
                </span>
              </label>
            </template>

            <template v-else>
              <p class="modal-hint">
                为 <code>{{ modalUser?.username }}</code> 设置新密码，提交后立即生效。
              </p>
              <label class="field modal-field-full is-required">
                <span>新密码</span>
                <input
                  v-model="form.password"
                  type="password"
                  autocomplete="new-password"
                  :minlength="VALIDATION_LIMITS.passwordMin"
                  :maxlength="VALIDATION_LIMITS.passwordMax"
                  placeholder="输入新的登录密码"
                />
              </label>
            </template>

            <div v-if="modalMode === 'create' || modalMode === 'edit'" class="field modal-field-full">
              <span>RBAC 角色</span>
              <div v-if="roles.length > 0" class="role-check-grid">
                <label
                  v-for="role in roles"
                  :key="role.id"
                  class="checkbox-field"
                >
                  <input
                    v-model="form.role_ids"
                    type="checkbox"
                    :value="role.id"
                  />
                  <span>{{ role.display_name }}</span>
                </label>
              </div>
              <p v-else class="field-hint">暂无可分配角色，请先在角色与权限中创建角色。</p>
            </div>
          </div>

          <div class="modal-actions toolbar-actions">
            <button type="submit" class="btn btn-primary" :disabled="submitting">
              {{ submitting ? "提交中..." : "保存" }}
            </button>
            <button
              type="button"
              class="btn btn-ghost"
              :disabled="submitting"
              @click="closeModal"
            >
              取消
            </button>
          </div>
        </form>
      </div>
    </div>
  </section>
</template>

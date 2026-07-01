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

type UserForm = {
  username: string;
  display_name: string;
  nickname: string;
  avatar_url: string;
  email: string;
  phone: string;
  department: string;
  title: string;
  password: string;
  confirm_password: string;
  role_ids: string[];
  disabled: boolean;
};

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
const statusFilter = ref<"all" | "active" | "disabled" | "pending" | "rejected">("all");
const roleFilter = ref<string>("");

const filteredUsers = computed(() =>
  users.value.filter((user) => {
    if (search.value) {
      const q = search.value.toLowerCase();
      const haystack =
        [
          user.username,
          user.display_name,
          user.nickname,
          user.email,
          user.phone,
          user.department,
          user.title,
        ]
          .filter(Boolean)
          .join(" ")
          .toLowerCase();
      if (!haystack.includes(q)) {
        return false;
      }
    }
    const status = userStatus(user).key;
    if (statusFilter.value !== "all" && status !== statusFilter.value) {
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
const form = ref<UserForm>({
  username: "",
  display_name: "",
  nickname: "",
  avatar_url: "",
  email: "",
  phone: "",
  department: "",
  title: "",
  password: "",
  confirm_password: "",
  role_ids: [] as string[],
  disabled: false,
});

function emptyUserForm(): UserForm {
  return {
    username: "",
    display_name: "",
    nickname: "",
    avatar_url: "",
    email: "",
    phone: "",
    department: "",
    title: "",
    password: "",
    confirm_password: "",
    role_ids: [],
    disabled: false,
  };
}

function optionalText(value: string) {
  const trimmed = value.trim();
  return trimmed || null;
}

function userProfilePayload(source: UserForm | AdminUserResponse) {
  return {
    nickname: optionalText(source.nickname ?? ""),
    avatar_url: optionalText(source.avatar_url ?? ""),
    phone: optionalText(source.phone ?? ""),
    department: optionalText(source.department ?? ""),
    title: optionalText(source.title ?? ""),
  };
}

function validatePasswordPair(required: boolean) {
  const password = form.value.password;
  const confirmPassword = form.value.confirm_password;
  if (!required && !password && !confirmPassword) {
    return true;
  }
  if (!password || !confirmPassword) {
    ui.setError("请填写密码和确认密码");
    return false;
  }
  if (password !== confirmPassword) {
    ui.setError("两次输入的密码不一致");
    return false;
  }
  if (password.length < VALIDATION_LIMITS.passwordMin) {
    ui.setError(`密码至少需要 ${VALIDATION_LIMITS.passwordMin} 位`);
    return false;
  }
  return true;
}

function openCreate() {
  modalMode.value = "create";
  modalUser.value = null;
  form.value = emptyUserForm();
}

function userHasDisabledRole(user: AdminUserResponse) {
  return (userRoleIds.value[user.id] ?? []).some((roleId) => disabledRoleIds.value.has(roleId));
}

function userUnavailable(user: AdminUserResponse) {
  return user.disabled || userHasDisabledRole(user);
}

function userStatus(user: AdminUserResponse): {
  key: "active" | "pending" | "rejected" | "disabled";
  tone: "good" | "neutral" | "warn" | "danger";
  label: string;
} {
  // Account-status driven by the registration / approval workflow takes
  // precedence over the legacy `disabled` flag.
  if (user.status === "pending") {
    return { key: "pending", tone: "warn", label: "待审核" };
  }
  if (user.status === "rejected") {
    return { key: "rejected", tone: "danger", label: "已拒绝" };
  }
  if (user.disabled) {
    return { key: "disabled", tone: "neutral", label: "已禁用" };
  }
  if (userHasDisabledRole(user)) {
    return { key: "disabled", tone: "neutral", label: "角色已禁用" };
  }
  return { key: "active", tone: "good", label: "启用" };
}

function openEdit(user: AdminUserResponse) {
  modalMode.value = "edit";
  modalUser.value = user;
  form.value = {
    username: user.username,
    display_name: user.display_name,
    nickname: user.nickname ?? "",
    avatar_url: user.avatar_url ?? "",
    email: user.email,
    phone: user.phone ?? "",
    department: user.department ?? "",
    title: user.title ?? "",
    password: "",
    confirm_password: "",
    role_ids: [...(userRoleIds.value[user.id] ?? [])],
    disabled: user.disabled,
  };
  closeMenu();
}

function openResetPassword(user: AdminUserResponse) {
  modalMode.value = "reset-password";
  modalUser.value = user;
  form.value.password = "";
  form.value.confirm_password = "";
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
      api.admin.listAdminUsers(),
      api.admin.listAdminRoles(),
    ]);
    users.value = userResponse.users;
    roles.value = roleResponse.roles;
    const rolePairs = await Promise.all(
      userResponse.users.map(async (user) => [
        user.id,
        (await api.admin.getAdminUserRoles(user.id)).roles.map((role) => role.id),
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
  if (!validatePasswordPair(true)) {
    return;
  }
  submitting.value = true;
  try {
    const created = await api.admin.createAdminUser({
      username: form.value.username.trim(),
      display_name: form.value.display_name.trim() || form.value.username.trim(),
      email: form.value.email.trim(),
      ...userProfilePayload(form.value),
      password: form.value.password,
      role_ids: form.value.role_ids,
    });
    if (form.value.role_ids.length > 0) {
      await api.admin.updateAdminUserRoles(created.id, {
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
  const shouldUpdatePassword = Boolean(form.value.password || form.value.confirm_password);
  if (!validatePasswordPair(false)) {
    return;
  }
  submitting.value = true;
  try {
    const userId = modalUser.value.id;
    await api.admin.updateAdminUser(userId, {
      display_name: form.value.display_name.trim() || modalUser.value.username,
      email: form.value.email.trim(),
      ...userProfilePayload(form.value),
      disabled: form.value.disabled,
    });
    await api.admin.updateAdminUserRoles(userId, {
      role_ids: form.value.role_ids,
    });
    if (shouldUpdatePassword) {
      await api.admin.resetAdminUserPassword(userId, {
        password: form.value.password,
      });
    }
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
  if (!validatePasswordPair(true)) {
    return;
  }
  submitting.value = true;
  try {
    await api.admin.resetAdminUserPassword(modalUser.value.id, {
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
      await api.admin.updateAdminUser(user.id, {
        display_name: user.display_name,
        email: user.email,
        ...userProfilePayload(user),
        disabled: false,
      });
    } else {
      await api.admin.disableAdminUser(user.id);
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
    await api.admin.deleteAdminUser(user.id);
    ui.setSuccess(`已删除用户 ${user.username}`);
    closeMenu();
    await loadUsers();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

async function approveUser(user: AdminUserResponse) {
  closeMenu();
  const confirmed = await ui.confirm({
    tone: "info",
    title: "通过注册申请",
    message: `确认通过 ${user.username} 的注册申请？通过后该账号即可登录。`,
    confirmLabel: "通过",
  });
  if (!confirmed) {
    return;
  }
  try {
    await api.admin.approveAdminUser(user.id);
    ui.setSuccess(`已通过 ${user.username} 的注册申请`);
    await loadUsers();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

async function rejectUser(user: AdminUserResponse) {
  closeMenu();
  const confirmed = await ui.confirm({
    tone: "danger",
    title: "拒绝注册申请",
    message: `确认拒绝 ${user.username} 的注册申请？拒绝后该账号将无法登录，除非管理员重新激活。`,
    confirmLabel: "拒绝",
  });
  if (!confirmed) {
    return;
  }
  try {
    await api.admin.rejectAdminUser(user.id);
    ui.setSuccess(`已拒绝 ${user.username} 的注册申请`);
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
    <div class="admin-table-panel">
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
              placeholder="按用户名 / 显示名 / 邮箱 / 手机 / 部门搜索"
            />
          </label>
          <label class="field select-field filter-field">
            <span>状态</span>
            <select v-model="statusFilter">
              <option value="all">全部状态</option>
              <option value="active">启用</option>
              <option value="pending">待审核</option>
              <option value="rejected">已拒绝</option>
              <option value="disabled">已禁用</option>
            </select>
          </label>
          <label class="field select-field filter-field">
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
                    v-if="user.status === 'pending'"
                    class="btn-icon-only"
                    title="通过注册申请"
                    @click="approveUser(user)"
                  >
                    <Icon name="check" :size="16" />
                  </button>
                  <button
                    v-if="user.status === 'pending'"
                    class="btn-icon-only"
                    title="拒绝注册申请"
                    @click="rejectUser(user)"
                  >
                    <Icon name="ban" :size="16" />
                  </button>
                  <button
                    class="btn-icon-only"
                    title="编辑"
                    @click="openEdit(user)"
                  >
                    <Icon name="edit" :size="16" />
                  </button>
                  <button
                    v-if="user.status !== 'pending'"
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
                        v-if="user.status === 'pending'"
                        class="action-menu-item"
                        @click="approveUser(user)"
                      >
                        <Icon name="check" :size="14" />
                        通过注册申请
                      </button>
                      <button
                        v-if="user.status === 'pending'"
                        class="action-menu-item"
                        @click="rejectUser(user)"
                      >
                        <Icon name="ban" :size="14" />
                        拒绝注册申请
                      </button>
                      <button
                        v-if="user.status !== 'pending'"
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
            <tr v-if="filteredUsers.length === 0" class="table-empty-row">
              <td colspan="8">
                <div class="empty-state">暂无用户数据</div>
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
            <template v-if="modalMode === 'create' || modalMode === 'edit'">
              <section class="form-section modal-form-section">
                <div class="form-section-header">
                  <span class="form-section-icon info"><Icon name="user" :size="16" /></span>
                  <h4>基本信息</h4>
                </div>
                <div class="modal-section-grid">
                  <label class="field" :class="{ 'is-required': modalMode === 'create' }">
                    <span>用户名</span>
                    <input
                      v-if="modalMode === 'create'"
                      v-model="form.username"
                      name="username"
                      autocomplete="off"
                      :minlength="VALIDATION_LIMITS.usernameMin"
                      :maxlength="VALIDATION_LIMITS.usernameMax"
                      :pattern="USERNAME_PATTERN"
                      placeholder="登录账号，必须唯一"
                    />
                    <input v-else name="username" :value="form.username" disabled />
                  </label>
                  <label class="field is-required">
                    <span>显示名</span>
                    <input
                      v-model="form.display_name"
                      name="display_name"
                      :autocomplete="modalMode === 'create' ? 'off' : undefined"
                      :minlength="VALIDATION_LIMITS.displayNameMin"
                      :maxlength="VALIDATION_LIMITS.displayNameMax"
                      :placeholder="modalMode === 'create' ? '页面展示名称，例如 张三' : '页面展示名称'"
                    />
                  </label>
                  <label class="field is-required">
                    <span>邮箱</span>
                    <input
                      v-model="form.email"
                      name="email"
                      type="email"
                      :autocomplete="modalMode === 'create' ? 'off' : undefined"
                      :minlength="VALIDATION_LIMITS.emailMin"
                      :maxlength="VALIDATION_LIMITS.emailMax"
                      placeholder="用户联系邮箱，例如 user@example.com"
                    />
                  </label>
                  <label class="field">
                    <span>昵称</span>
                    <input
                      v-model="form.nickname"
                      name="nickname"
                      :autocomplete="modalMode === 'create' ? 'off' : undefined"
                      :maxlength="VALIDATION_LIMITS.displayNameMax"
                      placeholder="用户昵称，可留空"
                    />
                  </label>
                  <label class="field">
                    <span>头像 URL</span>
                    <input
                      v-model="form.avatar_url"
                      name="avatar_url"
                      type="url"
                      :autocomplete="modalMode === 'create' ? 'off' : undefined"
                      :maxlength="VALIDATION_LIMITS.urlMax"
                      placeholder="头像图片地址，可留空"
                    />
                  </label>
                  <label class="field">
                    <span>手机号</span>
                    <input
                      v-model="form.phone"
                      name="phone"
                      :autocomplete="modalMode === 'create' ? 'off' : undefined"
                      :maxlength="VALIDATION_LIMITS.phoneMax"
                      placeholder="便于联系，可留空"
                    />
                  </label>
                  <label class="field">
                    <span>部门</span>
                    <input
                      v-model="form.department"
                      name="department"
                      :autocomplete="modalMode === 'create' ? 'off' : undefined"
                      :maxlength="VALIDATION_LIMITS.departmentMax"
                      placeholder="例如：内核组"
                    />
                  </label>
                  <label class="field">
                    <span>职位</span>
                    <input
                      v-model="form.title"
                      name="title"
                      :autocomplete="modalMode === 'create' ? 'off' : undefined"
                      :maxlength="VALIDATION_LIMITS.titleMax"
                      placeholder="例如：嵌入式工程师"
                    />
                  </label>
                </div>
              </section>

              <section class="form-section modal-form-section">
                <div class="form-section-header">
                  <span class="form-section-icon password"><Icon name="key" :size="16" /></span>
                  <h4>密码</h4>
                </div>
                <div class="modal-section-grid">
                  <label class="field" :class="{ 'is-required': modalMode === 'create' }">
                    <span>密码</span>
                    <input
                      v-model="form.password"
                      name="password"
                      type="password"
                      autocomplete="new-password"
                      :minlength="VALIDATION_LIMITS.passwordMin"
                      :maxlength="VALIDATION_LIMITS.passwordMax"
                      :placeholder="modalMode === 'create' ? '初始密码，建议至少 8 位' : '留空表示不修改密码'"
                    />
                  </label>
                  <label class="field" :class="{ 'is-required': modalMode === 'create' }">
                    <span>确认密码</span>
                    <input
                      v-model="form.confirm_password"
                      name="confirm_password"
                      type="password"
                      autocomplete="new-password"
                      :minlength="VALIDATION_LIMITS.passwordMin"
                      :maxlength="VALIDATION_LIMITS.passwordMax"
                      :placeholder="modalMode === 'create' ? '再次输入相同密码' : '留空表示不修改密码'"
                    />
                  </label>
                </div>
              </section>

              <section class="form-section modal-form-section">
                <div class="form-section-header">
                  <span class="form-section-icon roles"><Icon name="shield" :size="16" /></span>
                  <h4>系统角色</h4>
                </div>
                <div class="modal-section-grid">
                  <label v-if="modalMode === 'edit'" class="toggle-field modal-field-full">
                    <span class="toggle-switch">
                      <input v-model="form.disabled" type="checkbox" />
                      <span class="toggle-track" />
                      <span class="toggle-knob" />
                    </span>
                    <span class="toggle-label">
                      {{ form.disabled ? "已禁用登录" : "允许登录" }}
                    </span>
                  </label>
                  <div class="field modal-field-full">
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
              </section>
            </template>

            <template v-else>
              <p class="modal-hint">
                为 <code>{{ modalUser?.username }}</code> 设置新密码，提交后立即生效。
              </p>
              <label class="field modal-field-full is-required">
                <span>新密码</span>
                <input
                  v-model="form.password"
                  name="password"
                  type="password"
                  autocomplete="new-password"
                  :minlength="VALIDATION_LIMITS.passwordMin"
                  :maxlength="VALIDATION_LIMITS.passwordMax"
                  placeholder="输入新的登录密码"
                />
              </label>
              <label class="field modal-field-full is-required">
                <span>确认新密码</span>
                <input
                  v-model="form.confirm_password"
                  name="confirm_password"
                  type="password"
                  autocomplete="new-password"
                  :minlength="VALIDATION_LIMITS.passwordMin"
                  :maxlength="VALIDATION_LIMITS.passwordMax"
                  placeholder="再次输入相同密码"
                />
              </label>
            </template>
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

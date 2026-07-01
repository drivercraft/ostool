<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { ROLE_NAME_PATTERN, VALIDATION_LIMITS } from "@/constants/validation";
import { useUiStore } from "@/stores/ui";
import type { AdminPermissionResponse, AdminRoleResponse } from "@/types/api";

interface PermissionGroup {
  key: string;
  title: string;
  description: string;
  permissions: AdminPermissionResponse[];
}

const ui = useUiStore();
const route = useRoute();
const router = useRouter();
const loading = ref(true);
const saving = ref(false);
const roles = ref<AdminRoleResponse[]>([]);
const permissions = ref<AdminPermissionResponse[]>([]);
const selectedRoleId = ref<string | null>(null);
const openMenuRoleId = ref<string | null>(null);
const menuPosition = ref({ top: 0, left: 0 });
const search = ref("");
const typeFilter = ref<"all" | "system" | "custom">("all");
const statusFilter = ref<"all" | "active" | "disabled">("all");
const form = ref({
  name: "",
  display_name: "",
  description: "",
  permission_ids: [] as string[],
});

const selectedRole = computed(
  () => roles.value.find((role) => role.id === selectedRoleId.value) ?? null,
);
const editing = computed(() =>
  route.name === "admin-user-role-new" || route.name === "admin-user-role-edit",
);
const editingExistingRole = computed(() => route.name === "admin-user-role-edit");

const filteredRoles = computed(() =>
  roles.value.filter((role) => {
    if (typeFilter.value === "system" && !role.system) {
      return false;
    }
    if (typeFilter.value === "custom" && role.system) {
      return false;
    }
    if (statusFilter.value === "active" && role.disabled) {
      return false;
    }
    if (statusFilter.value === "disabled" && !role.disabled) {
      return false;
    }
    const query = search.value.trim().toLowerCase();
    if (!query) {
      return true;
    }
    return [
      role.display_name,
      role.name,
      role.description,
      ...role.permissions.map((permission) => permission.code),
    ].some((value) => value.toLowerCase().includes(query));
  }),
);

const permissionGroups = computed<PermissionGroup[]>(() => {
  const labels: Record<string, { title: string; description: string }> = {
    overview: { title: "概览", description: "运行总览与平台统计" },
    users: { title: "用户管理", description: "用户账号与角色分配" },
    roles: { title: "角色与权限", description: "角色创建、编辑与权限配置" },
    boards: { title: "开发板管理", description: "开发板配置与运行状态" },
    dtbs: { title: "DTB 管理", description: "DTB 文件与元数据" },
    leases: { title: "租赁情况", description: "租赁记录、预约时间段与会话启动" },
    sessions: { title: "租约会话", description: "租约会话记录与历史数据" },
    tftp: { title: "TFTP 配置", description: "TFTP 配置、状态与同步" },
    server: { title: "服务器配置", description: "服务运行参数" },
    site: { title: "站点设置", description: "站点展示与租赁策略" },
    serial_ports: { title: "串口资源", description: "服务器可用串口" },
    network_interfaces: { title: "网络接口", description: "服务器网络接口" },
    permissions: { title: "权限目录", description: "系统内置权限列表" },
  };
  const order = [
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
  ];
  const actionOrder = [
    "read",
    "create",
    "update",
    "start",
    "release",
    "heartbeat",
    "reconcile",
    "delete",
  ];
  const map = new Map<string, AdminPermissionResponse[]>();
  for (const permission of permissions.value) {
    const key = permission.code.split(".")[0] || "other";
    map.set(key, [...(map.get(key) ?? []), permission]);
  }
  return Array.from(map.entries())
    .sort(([left], [right]) => {
      const leftIndex = order.indexOf(left);
      const rightIndex = order.indexOf(right);
      return (leftIndex === -1 ? Number.MAX_SAFE_INTEGER : leftIndex)
        - (rightIndex === -1 ? Number.MAX_SAFE_INTEGER : rightIndex)
        || left.localeCompare(right);
    })
    .map(([key, items]) => ({
      key,
      title: labels[key]?.title ?? key,
      description: labels[key]?.description ?? "系统权限",
      permissions: [...items].sort((left, right) => {
        const leftAction = left.code.split(".").slice(1).join(".");
        const rightAction = right.code.split(".").slice(1).join(".");
        const leftIndex = actionOrder.indexOf(leftAction);
        const rightIndex = actionOrder.indexOf(rightAction);
        return (leftIndex === -1 ? Number.MAX_SAFE_INTEGER : leftIndex)
          - (rightIndex === -1 ? Number.MAX_SAFE_INTEGER : rightIndex)
          || left.code.localeCompare(right.code);
      }),
    }));
});

function groupPermissionIds(group: PermissionGroup) {
  return group.permissions.map((permission) => permission.id);
}

function selectedPermissionCount(group: PermissionGroup) {
  const selected = new Set(form.value.permission_ids);
  return groupPermissionIds(group).filter((permissionId) => selected.has(permissionId)).length;
}

function isPermissionGroupSelected(group: PermissionGroup) {
  return group.permissions.length > 0 && selectedPermissionCount(group) === group.permissions.length;
}

function isPermissionGroupPartial(group: PermissionGroup) {
  const count = selectedPermissionCount(group);
  return count > 0 && count < group.permissions.length;
}

function togglePermissionGroup(group: PermissionGroup, checked: boolean) {
  const groupIds = new Set(groupPermissionIds(group));
  const next = new Set(form.value.permission_ids);
  if (checked) {
    groupIds.forEach((permissionId) => next.add(permissionId));
  } else {
    groupIds.forEach((permissionId) => next.delete(permissionId));
  }
  form.value.permission_ids = permissions.value
    .filter((permission) => next.has(permission.id))
    .map((permission) => permission.id);
}

function resetForm() {
  selectedRoleId.value = null;
  form.value = {
    name: "",
    display_name: "",
    description: "",
    permission_ids: [],
  };
}

function openCreate() {
  closeMenu();
  void router.push({ name: "admin-user-role-new" });
}

function editRole(role: AdminRoleResponse) {
  closeMenu();
  void router.push({ name: "admin-user-role-edit", params: { roleId: role.id } });
}

function fillRoleForm(role: AdminRoleResponse) {
  selectedRoleId.value = role.id;
  form.value = {
    name: role.name,
    display_name: role.display_name,
    description: role.description,
    permission_ids: role.permissions.map((permission) => permission.id),
  };
}

function cancelEdit() {
  resetForm();
  void router.push({ name: "admin-user-roles" });
}

function toggleMenu(roleId: string, event: MouseEvent) {
  if (openMenuRoleId.value === roleId) {
    closeMenu();
    return;
  }
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  menuPosition.value = {
    top: rect.bottom + 6,
    left: Math.max(12, rect.right - 180),
  };
  openMenuRoleId.value = roleId;
}

function closeMenu() {
  openMenuRoleId.value = null;
}

function onDocumentClick(event: MouseEvent) {
  const target = event.target as HTMLElement | null;
  if (target && !target.closest(".row-action-menu") && !target.closest(".action-menu")) {
    closeMenu();
  }
}

function syncEditorFromRoute() {
  if (route.name === "admin-user-role-new") {
    resetForm();
    closeMenu();
    return;
  }
  if (route.name === "admin-user-role-edit") {
    const roleId = typeof route.params.roleId === "string" ? route.params.roleId : "";
    const role = roles.value.find((item) => item.id === roleId);
    if (role) {
      fillRoleForm(role);
      closeMenu();
      return;
    }
    if (!loading.value) {
      ui.setError("未找到要编辑的角色");
      void router.replace({ name: "admin-user-roles" });
    }
    return;
  }
  resetForm();
}

async function loadRbac() {
  loading.value = true;
  try {
    const [roleResponse, permissionResponse] = await Promise.all([
      api.admin.listAdminRoles(),
      api.admin.listAdminPermissions(),
    ]);
    roles.value = roleResponse.roles;
    permissions.value = permissionResponse.permissions;
    if (selectedRoleId.value && !roles.value.some((role) => role.id === selectedRoleId.value)) {
      resetForm();
    }
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
    syncEditorFromRoute();
  }
}

async function saveRole() {
  if (!form.value.name.trim() || !form.value.display_name.trim()) {
    ui.setError("角色标识和显示名不能为空");
    return;
  }
  if (form.value.name.trim().length < VALIDATION_LIMITS.roleNameMin) {
    ui.setError(`角色标识至少需要 ${VALIDATION_LIMITS.roleNameMin} 个字符`);
    return;
  }
  if (form.value.description.trim().length > VALIDATION_LIMITS.descriptionMax) {
    ui.setError(`角色描述不能超过 ${VALIDATION_LIMITS.descriptionMax} 个字符`);
    return;
  }
  if (editingExistingRole.value && !selectedRole.value) {
    ui.setError("未找到要编辑的角色");
    return;
  }
  saving.value = true;
  try {
    if (selectedRole.value) {
      await api.admin.updateAdminRole(selectedRole.value.id, {
        display_name: form.value.display_name.trim(),
        description: form.value.description.trim(),
        permission_ids: form.value.permission_ids,
      });
      ui.setSuccess("角色已更新");
    } else {
      await api.admin.createAdminRole({
        name: form.value.name.trim(),
        display_name: form.value.display_name.trim(),
        description: form.value.description.trim(),
        permission_ids: form.value.permission_ids,
      });
      ui.setSuccess("角色已创建");
    }
    resetForm();
    await router.push({ name: "admin-user-roles" });
    await loadRbac();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    saving.value = false;
  }
}

async function deleteRole(role: AdminRoleResponse) {
  if (role.system) {
    return;
  }
  const confirmed = await ui.confirm({
    tone: "danger",
    title: "删除角色",
    message: `确认删除角色 ${role.display_name}？`,
    confirmLabel: "删除",
  });
  if (!confirmed) {
    return;
  }
  try {
    await api.admin.deleteAdminRole(role.id);
    ui.setSuccess("角色已删除");
    if (selectedRoleId.value === role.id) {
      resetForm();
    }
    closeMenu();
    await loadRbac();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

async function toggleRoleDisabled(role: AdminRoleResponse) {
  if (role.system) {
    return;
  }
  closeMenu();
  const action = role.disabled ? "启用" : "禁用";
  const confirmed = await ui.confirm({
    tone: role.disabled ? "info" : "danger",
    title: `${action}角色`,
    message: `确认${action}角色 ${role.display_name}？`,
    confirmLabel: action,
  });
  if (!confirmed) {
    return;
  }
  try {
    const updated = await api.admin.disableAdminRole(role.id, {
      disabled: !role.disabled,
    });
    roles.value = roles.value.map((item) => (item.id === updated.id ? updated : item));
    ui.setSuccess(updated.disabled ? "角色已禁用" : "角色已启用");
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

onMounted(() => {
  document.addEventListener("click", onDocumentClick);
  void loadRbac();
});
onUnmounted(() => document.removeEventListener("click", onDocumentClick));

watch(
  () => [route.name, route.params.roleId],
  () => syncEditorFromRoute(),
);
</script>

<template>
  <section
    class="page-grid user-management-page roles-page"
    :class="editing ? 'admin-editor-page' : 'admin-list-page'"
  >
    <template v-if="editing">
      <div class="admin-editor-panel">
        <form class="admin-editor-form" @submit.prevent="saveRole">
          <div class="admin-editor-body">
            <div class="role-basic-fields">
              <label class="field is-required">
                <span>角色名称</span>
                <input
                  v-model="form.display_name"
                  :minlength="VALIDATION_LIMITS.displayNameMin"
                  :maxlength="VALIDATION_LIMITS.displayNameMax"
                  placeholder="例如 开发人员"
                />
              </label>
              <label class="field is-required">
                <span>角色标识</span>
                <input
                  v-model="form.name"
                  :disabled="Boolean(selectedRole)"
                  :minlength="VALIDATION_LIMITS.roleNameMin"
                  :maxlength="VALIDATION_LIMITS.roleNameMax"
                  :pattern="ROLE_NAME_PATTERN"
                  placeholder="仅小写字母、数字、_ 或 -，例如 developer"
                />
              </label>
              <label class="field role-field-full">
                <span>角色描述（选填）</span>
                <input
                  v-model="form.description"
                  :maxlength="VALIDATION_LIMITS.descriptionMax"
                  placeholder="说明该角色的使用范围"
                />
              </label>
            </div>

            <div class="role-permission-section">
              <div class="role-permission-section-head">
                <div>
                  <h4>功能模块</h4>
                </div>
                <span class="muted">已选择 {{ form.permission_ids.length }} / {{ permissions.length }}</span>
              </div>

              <div class="permission-matrix">
                <section
                  v-for="group in permissionGroups"
                  :key="group.key"
                  class="permission-matrix-row"
                >
                  <div class="permission-matrix-module">
                    <strong>{{ group.title }}</strong>
                    <span>{{ group.description }}</span>
                    <label class="toggle-field permission-group-toggle">
                      <span class="toggle-switch">
                        <input
                          type="checkbox"
                          :aria-label="`${group.title}权限全选`"
                          :checked="isPermissionGroupSelected(group)"
                          :indeterminate="isPermissionGroupPartial(group)"
                          @change="togglePermissionGroup(group, ($event.target as HTMLInputElement).checked)"
                        />
                        <span class="toggle-track" />
                        <span class="toggle-knob" />
                      </span>
                    </label>
                  </div>
                  <div class="permission-matrix-options">
                    <label
                      v-for="permission in group.permissions"
                      :key="permission.id"
                      class="permission-option"
                    >
                      <input
                        v-model="form.permission_ids"
                        type="checkbox"
                        :value="permission.id"
                      />
                      <span>
                        <strong>{{ permission.name }}</strong>
                        <small>{{ permission.code }}</small>
                        <small>{{ permission.description }}</small>
                      </span>
                    </label>
                  </div>
                </section>
              </div>
            </div>
          </div>

          <div class="admin-editor-actions">
            <button type="submit" class="btn btn-primary" :disabled="saving">
              {{ saving ? "保存中..." : selectedRole ? "保存角色" : "创建角色" }}
            </button>
            <button type="button" class="btn btn-ghost" :disabled="saving" @click="cancelEdit">取消</button>
          </div>
        </form>
      </div>
    </template>

    <template v-else>
      <div class="admin-table-panel role-table-panel">
        <div class="admin-toolbar">
          <div class="admin-toolbar-left">
            <button class="btn btn-primary" @click="openCreate">新增角色</button>
          </div>
          <div class="admin-toolbar-right">
            <label class="search-field">
              <Icon name="search" :size="16" />
              <input v-model="search" type="search" maxlength="128" placeholder="搜索角色 / 标识 / 描述" />
            </label>
            <label class="field select-field filter-field">
              <span>类型</span>
              <select v-model="typeFilter">
                <option value="all">全部类型</option>
                <option value="system">系统角色</option>
                <option value="custom">自定义角色</option>
              </select>
            </label>
            <label class="field select-field filter-field">
              <span>状态</span>
              <select v-model="statusFilter">
                <option value="all">全部状态</option>
                <option value="active">启用</option>
                <option value="disabled">已禁用</option>
              </select>
            </label>
          </div>
        </div>

        <div v-if="loading" class="empty-state">正在加载角色...</div>
        <div v-else class="table-scroll">
          <table class="data-table roles-table">
            <thead>
              <tr>
                <th class="col-index">序号</th>
                <th>角色</th>
                <th>标识</th>
                <th>类型</th>
                <th>状态</th>
                <th>用户数量</th>
                <th>描述</th>
                <th class="col-actions">操作</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(role, index) in filteredRoles" :key="role.id">
                <td class="col-index">{{ index + 1 }}</td>
                <td>{{ role.display_name }}</td>
                <td>{{ role.name }}</td>
                <td>
                  <StatusPill
                    :tone="role.system ? 'neutral' : 'good'"
                    :label="role.system ? '系统角色' : '自定义角色'"
                  />
                </td>
                <td>
                  <StatusPill
                    :tone="role.disabled ? 'neutral' : 'good'"
                    :label="role.disabled ? '已禁用' : '启用'"
                  />
                </td>
                <td>{{ role.user_count }}</td>
                <td class="muted">{{ role.description || "无描述" }}</td>
                <td class="col-actions">
                  <div class="row-actions">
                    <button
                      class="btn-icon-only"
                      title="编辑"
                      @click="editRole(role)"
                    >
                      <Icon name="edit" :size="16" />
                    </button>
                    <button
                      class="btn-icon-only"
                      :title="role.disabled ? '启用' : '禁用'"
                      :disabled="role.system"
                      @click="toggleRoleDisabled(role)"
                    >
                      <Icon :name="role.disabled ? 'check' : 'ban'" :size="16" />
                    </button>
                    <div class="row-action-menu">
                      <button
                        class="btn-icon-only"
                        title="更多"
                        :aria-expanded="openMenuRoleId === role.id"
                        @click.stop="toggleMenu(role.id, $event)"
                      >
                        <Icon name="more-vertical" :size="16" />
                      </button>
                    </div>
                    <Teleport to="body">
                      <div
                        v-if="openMenuRoleId === role.id"
                        class="action-menu action-menu--floating"
                        :style="{ top: `${menuPosition.top}px`, left: `${menuPosition.left}px` }"
                      >
                        <button
                          class="action-menu-item"
                          @click="editRole(role)"
                        >
                          <Icon name="edit" :size="14" />
                          编辑角色
                        </button>
                        <button
                          class="action-menu-item"
                          :disabled="role.system"
                          @click="deleteRole(role)"
                        >
                          <Icon name="trash" :size="14" />
                          删除角色
                        </button>
                      </div>
                    </Teleport>
                  </div>
                </td>
              </tr>
              <tr v-if="filteredRoles.length === 0" class="table-empty-row">
                <td colspan="8">
                  <div class="empty-state">暂无角色数据</div>
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <div v-if="!loading" class="table-statusbar">
          <span>{{ filteredRoles.length === 0 ? "暂无分页" : "第 1 / 共 1 页" }}</span>
          <span>本页 {{ filteredRoles.length }} 条</span>
          <span>筛选后 {{ filteredRoles.length }} 条 / 共 {{ roles.length }} 条</span>
        </div>
      </div>

    </template>
  </section>
</template>

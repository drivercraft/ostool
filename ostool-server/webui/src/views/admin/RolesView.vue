<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from "vue";

import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { AdminPermissionResponse, AdminRoleResponse } from "@/types/api";

interface PermissionGroup {
  key: string;
  title: string;
  description: string;
  permissions: AdminPermissionResponse[];
}

const ui = useUiStore();
const loading = ref(true);
const saving = ref(false);
const roles = ref<AdminRoleResponse[]>([]);
const permissions = ref<AdminPermissionResponse[]>([]);
const selectedRoleId = ref<string | null>(null);
const editing = ref(false);
const showPermissionInfo = ref(false);
const openMenuRoleId = ref<string | null>(null);
const menuPosition = ref({ top: 0, left: 0 });
const search = ref("");
const typeFilter = ref<"all" | "system" | "custom">("all");
const form = ref({
  name: "",
  display_name: "",
  description: "",
  permission_ids: [] as string[],
});

const selectedRole = computed(
  () => roles.value.find((role) => role.id === selectedRoleId.value) ?? null,
);

const filteredRoles = computed(() =>
  roles.value.filter((role) => {
    if (typeFilter.value === "system" && !role.system) {
      return false;
    }
    if (typeFilter.value === "custom" && role.system) {
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
    resources: { title: "资源管理", description: "开发板、DTB 与 TFTP 配置" },
    rentals: { title: "租赁管理", description: "租赁情况与会话租约" },
    users: { title: "用户管理", description: "用户账号与角色分配" },
    roles: { title: "角色与权限", description: "角色创建、编辑与权限配置" },
    settings: { title: "系统设置", description: "服务运行配置" },
  };
  const map = new Map<string, AdminPermissionResponse[]>();
  for (const permission of permissions.value) {
    const key = permission.code.split(".")[0] || "other";
    map.set(key, [...(map.get(key) ?? []), permission]);
  }
  return Array.from(map.entries()).map(([key, items]) => ({
    key,
    title: labels[key]?.title ?? key,
    description: labels[key]?.description ?? "系统权限",
    permissions: items,
  }));
});

function rolesForPermission(permissionId: string) {
  return roles.value.filter((role) =>
    role.permissions.some((permission) => permission.id === permissionId),
  );
}

function resetForm() {
  selectedRoleId.value = null;
  editing.value = false;
  form.value = {
    name: "",
    display_name: "",
    description: "",
    permission_ids: [],
  };
}

function openCreate() {
  resetForm();
  closeMenu();
  editing.value = true;
  void nextTick(scrollEditorIntoView);
}

function editRole(role: AdminRoleResponse) {
  selectedRoleId.value = role.id;
  form.value = {
    name: role.name,
    display_name: role.display_name,
    description: role.description,
    permission_ids: role.permissions.map((permission) => permission.id),
  };
  closeMenu();
  editing.value = true;
  void nextTick(scrollEditorIntoView);
}

function cancelEdit() {
  resetForm();
}

function togglePermissionInfo() {
  showPermissionInfo.value = !showPermissionInfo.value;
}

function scrollEditorIntoView() {
  document.querySelector(".role-editor-page")?.scrollIntoView({ behavior: "smooth", block: "start" });
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

async function loadRbac() {
  loading.value = true;
  try {
    const [roleResponse, permissionResponse] = await Promise.all([
      api.listAdminRoles(),
      api.listAdminPermissions(),
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
  }
}

async function saveRole() {
  if (!form.value.name.trim() || !form.value.display_name.trim()) {
    ui.setError("角色标识和显示名不能为空");
    return;
  }
  saving.value = true;
  try {
    if (selectedRole.value) {
      await api.updateAdminRole(selectedRole.value.id, {
        display_name: form.value.display_name.trim(),
        description: form.value.description.trim(),
        permission_ids: form.value.permission_ids,
      });
      ui.setSuccess("角色已更新");
    } else {
      await api.createAdminRole({
        name: form.value.name.trim(),
        display_name: form.value.display_name.trim(),
        description: form.value.description.trim(),
        permission_ids: form.value.permission_ids,
      });
      ui.setSuccess("角色已创建");
    }
    resetForm();
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
    await api.deleteAdminRole(role.id);
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

onMounted(() => {
  document.addEventListener("click", onDocumentClick);
  void loadRbac();
});
onUnmounted(() => document.removeEventListener("click", onDocumentClick));
</script>

<template>
  <section class="page-grid user-management-page roles-page admin-list-page">
    <template v-if="editing">
      <div class="role-editor-page panel">
        <div class="role-editor-titlebar">
          <div>
            <h3>{{ selectedRole ? "编辑角色" : "新建角色" }}</h3>
            <p class="muted">配置角色基础信息，并为该角色勾选可访问的功能权限。</p>
          </div>
          <button class="btn btn-ghost btn-sm" type="button" @click="cancelEdit">返回列表</button>
        </div>

        <form class="role-editor-form" @submit.prevent="saveRole">
          <div class="role-basic-fields">
            <label class="field">
              <span>角色名称</span>
              <input v-model="form.display_name" placeholder="例如 开发人员" />
            </label>
            <label class="field">
              <span>角色标识</span>
              <input
                v-model="form.name"
                :disabled="Boolean(selectedRole)"
                placeholder="例如 developer"
              />
            </label>
            <label class="field role-field-full">
              <span>角色描述（选填）</span>
              <input v-model="form.description" placeholder="说明该角色的使用范围" />
            </label>
          </div>

          <div class="role-permission-section">
            <div class="role-permission-section-head">
              <div>
                <h4>功能模块</h4>
                <p class="muted">按模块勾选权限，保存后立即影响该角色下的用户访问范围。</p>
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
                      <small>{{ permission.description || permission.code }}</small>
                    </span>
                  </label>
                </div>
              </section>
            </div>
          </div>

          <div class="role-editor-actions">
            <button type="submit" class="btn btn-primary" :disabled="saving">
              {{ saving ? "保存中..." : selectedRole ? "保存角色" : "创建角色" }}
            </button>
            <button type="button" class="btn btn-ghost" :disabled="saving" @click="cancelEdit">取消</button>
          </div>
        </form>
      </div>
    </template>

    <template v-else>
      <div class="panel admin-table-panel role-table-panel">
        <div class="admin-toolbar">
          <div class="admin-toolbar-left">
            <button class="btn btn-primary" @click="openCreate">新增角色</button>
            <button class="btn btn-ghost btn-sm" @click="togglePermissionInfo">
              {{ showPermissionInfo ? "隐藏权限说明" : "权限说明" }}
            </button>
            <button class="btn btn-ghost btn-sm" @click="loadRbac">刷新</button>
          </div>
          <div class="admin-toolbar-right">
            <label class="search-field">
              <Icon name="search" :size="16" />
              <input v-model="search" type="search" placeholder="搜索角色 / 权限" />
            </label>
            <label class="field filter-field">
              <span>类型</span>
              <select v-model="typeFilter">
                <option value="all">全部类型</option>
                <option value="system">系统角色</option>
                <option value="custom">自定义角色</option>
              </select>
            </label>
          </div>
        </div>

        <div v-if="loading" class="empty-state">正在加载角色...</div>
        <div v-else-if="filteredRoles.length === 0" class="empty-state">没有符合条件的角色。</div>
        <div v-else class="table-scroll">
          <table class="data-table roles-table">
            <thead>
              <tr>
                <th class="col-index">序号</th>
                <th>角色</th>
                <th>标识</th>
                <th>类型</th>
                <th>权限</th>
                <th>描述</th>
                <th class="col-actions">操作</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(role, index) in filteredRoles" :key="role.id">
                <td class="col-index">{{ index + 1 }}</td>
                <td><strong>{{ role.display_name }}</strong></td>
                <td><code>{{ role.name }}</code></td>
                <td>
                  <StatusPill
                    :tone="role.system ? 'neutral' : 'good'"
                    :label="role.system ? '系统角色' : '自定义角色'"
                  />
                </td>
                <td>
                  <div class="role-chip-list">
                    <span
                      v-for="permission in role.permissions"
                      :key="permission.id"
                      class="tag-chip"
                    >
                      {{ permission.code }}
                    </span>
                    <span v-if="role.permissions.length === 0" class="muted">无</span>
                  </div>
                </td>
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
                      title="启用/禁用"
                      disabled
                    >
                      <Icon name="ban" :size="16" />
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
            </tbody>
          </table>
        </div>

        <div v-if="!loading" class="table-statusbar">
          <span>{{ filteredRoles.length === 0 ? "暂无分页" : "第 1 / 共 1 页" }}</span>
          <span>本页 {{ filteredRoles.length }} 条</span>
          <span>筛选后 {{ filteredRoles.length }} 条 / 共 {{ roles.length }} 条</span>
        </div>
      </div>

      <div v-if="showPermissionInfo" class="panel permission-catalog-panel">
        <div class="panel-heading admin-section-heading">
          <div>
            <h3>权限配置</h3>
            <p class="muted">查看系统内置权限，以及当前有哪些角色持有这些权限。</p>
          </div>
        </div>

        <div v-if="loading" class="empty-state">正在加载权限...</div>
        <div v-else class="permission-grid">
          <article
            v-for="permission in permissions"
            :key="permission.id"
            class="permission-card"
          >
            <div class="permission-card-code">
              <code>{{ permission.code }}</code>
            </div>
            <div>
              <h4>{{ permission.name }}</h4>
              <p class="muted">{{ permission.description }}</p>
            </div>
            <div class="permission-role-list">
              <span
                v-for="role in rolesForPermission(permission.id)"
                :key="role.id"
                class="tag-chip"
              >
                {{ role.display_name }}
              </span>
              <span v-if="rolesForPermission(permission.id).length === 0" class="muted">
                暂未分配给任何角色
              </span>
            </div>
          </article>
        </div>
      </div>
    </template>
  </section>
</template>

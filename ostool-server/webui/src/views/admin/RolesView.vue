<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { AdminPermissionResponse, AdminRoleResponse } from "@/types/api";

const ui = useUiStore();
const loading = ref(true);
const saving = ref(false);
const roles = ref<AdminRoleResponse[]>([]);
const permissions = ref<AdminPermissionResponse[]>([]);
const selectedRoleId = ref<string | null>(null);
const form = ref({
  name: "",
  display_name: "",
  description: "",
  permission_ids: [] as string[],
});

const selectedRole = computed(
  () => roles.value.find((role) => role.id === selectedRoleId.value) ?? null,
);

function resetForm() {
  selectedRoleId.value = null;
  form.value = {
    name: "",
    display_name: "",
    description: "",
    permission_ids: [],
  };
}

function editRole(role: AdminRoleResponse) {
  selectedRoleId.value = role.id;
  form.value = {
    name: role.name,
    display_name: role.display_name,
    description: role.description,
    permission_ids: role.permissions.map((permission) => permission.id),
  };
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
  if (role.system || !window.confirm(`确认删除角色 ${role.display_name}？`)) {
    return;
  }
  try {
    await api.deleteAdminRole(role.id);
    ui.setSuccess("角色已删除");
    if (selectedRoleId.value === role.id) {
      resetForm();
    }
    await loadRbac();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

onMounted(() => {
  void loadRbac();
});
</script>

<template>
  <div class="split-grid">
    <section class="panel">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">用户管理</p>
          <h3>用户角色</h3>
        </div>
        <button class="ghost-button compact-button" @click="loadRbac">刷新</button>
      </div>

      <div v-if="loading" class="empty-state">正在加载角色...</div>
      <div v-else class="role-list">
        <article v-for="role in roles" :key="role.id" class="role-card">
          <div>
            <div class="role-card-title">
              <strong>{{ role.display_name }}</strong>
              <StatusPill
                :tone="role.system ? 'neutral' : 'good'"
                :label="role.system ? '系统角色' : '自定义角色'"
              />
            </div>
            <code>{{ role.name }}</code>
            <p>{{ role.description || "无描述" }}</p>
          </div>
          <div class="resource-card-tags">
            <span
              v-for="permission in role.permissions"
              :key="permission.id"
              class="tag-chip"
            >
              {{ permission.code }}
            </span>
          </div>
          <div class="toolbar-actions">
            <button class="ghost-button compact-button" @click="editRole(role)">编辑</button>
            <button
              class="danger-button compact-button"
              :disabled="role.system"
              @click="deleteRole(role)"
            >
              删除
            </button>
          </div>
        </article>
      </div>
    </section>

    <section class="panel">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">RBAC</p>
          <h3>{{ selectedRole ? "编辑角色" : "创建角色" }}</h3>
        </div>
        <button class="ghost-button compact-button" @click="resetForm">新建</button>
      </div>

      <div class="dashboard-form">
        <label class="field">
          <span>角色标识</span>
          <input
            v-model="form.name"
            :disabled="Boolean(selectedRole)"
            placeholder="例如 board_operator"
          />
        </label>
        <label class="field">
          <span>显示名</span>
          <input v-model="form.display_name" placeholder="例如 开发板运维" />
        </label>
        <label class="field">
          <span>描述</span>
          <textarea v-model="form.description" rows="3" />
        </label>

        <div class="permission-check-grid">
          <label
            v-for="permission in permissions"
            :key="permission.id"
            class="checkbox-field"
          >
            <input
              v-model="form.permission_ids"
              type="checkbox"
              :value="permission.id"
            />
            <span>
              <strong>{{ permission.name }}</strong>
              <small>{{ permission.code }}</small>
            </span>
          </label>
        </div>

        <div class="toolbar-actions">
          <button class="primary-button" :disabled="saving" @click="saveRole">
            {{ saving ? "保存中..." : selectedRole ? "保存角色" : "创建角色" }}
          </button>
        </div>
      </div>
    </section>
  </div>
</template>

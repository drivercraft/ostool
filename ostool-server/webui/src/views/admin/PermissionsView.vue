<script setup lang="ts">
import { onMounted, ref } from "vue";

import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { AdminPermissionResponse, AdminRoleResponse } from "@/types/api";

const ui = useUiStore();
const loading = ref(true);
const permissions = ref<AdminPermissionResponse[]>([]);
const roles = ref<AdminRoleResponse[]>([]);

async function loadPermissions() {
  loading.value = true;
  try {
    const [permissionResponse, roleResponse] = await Promise.all([
      api.listAdminPermissions(),
      api.listAdminRoles(),
    ]);
    permissions.value = permissionResponse.permissions;
    roles.value = roleResponse.roles;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

function rolesForPermission(permissionId: string) {
  return roles.value.filter((role) =>
    role.permissions.some((permission) => permission.id === permissionId),
  );
}

onMounted(() => {
  void loadPermissions();
});
</script>

<template>
  <section class="panel">
    <div class="panel-heading">
      <div>
        <p class="eyebrow">用户管理</p>
        <h3>权限配置</h3>
      </div>
      <button class="ghost-button compact-button" @click="loadPermissions">刷新</button>
    </div>

    <div v-if="loading" class="empty-state">正在加载权限...</div>
    <div v-else class="resource-card-grid">
      <article
        v-for="permission in permissions"
        :key="permission.id"
        class="resource-card"
      >
        <div class="resource-card-header">
          <code class="resource-card-type">{{ permission.code }}</code>
        </div>
        <div>
          <h4>{{ permission.name }}</h4>
          <p class="muted">{{ permission.description }}</p>
        </div>
        <div class="resource-card-tags">
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
  </section>
</template>

<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import { formatLeaseTime } from "./useUserLeases";

const ui = useUiStore();
const auth = useAuthStore();
const router = useRouter();

const accountFields = computed(() => [
  ["用户名", auth.user?.username ?? "-"],
  ["显示名称", auth.user?.display_name ?? "-"],
  ["邮箱", auth.user?.email ?? "-"],
  ["手机号", auth.user?.phone ?? "-"],
  ["部门", auth.user?.department ?? "-"],
  ["职位", auth.user?.title ?? "-"],
  ["最后登录", auth.user?.last_login_at ? formatLeaseTime(auth.user.last_login_at) : "-"],
  ["角色", auth.user?.roles.map((role) => role.display_name || role.name).join("、") || "普通用户"],
]);

onMounted(() => {
  ui.clearMessages();
  if (!auth.isAuthenticated) {
    void router.replace("/login");
  }
});
</script>

<template>
  <section class="dashboard-account-card card">
    <div class="panel-heading compact dashboard-section-heading">
      <div>
        <h3>账户信息</h3>
        <p class="muted">当前登录账户的基础资料。</p>
      </div>
      <button class="btn btn-ghost btn-sm" type="button" disabled>
        <Icon name="key" :size="14" class="btn-icon" />
        修改密码
      </button>
    </div>
    <dl class="profile-dl dashboard-account-dl">
      <div v-for="[label, value] in accountFields" :key="label">
        <dt>{{ label }}</dt>
        <dd>{{ value }}</dd>
      </div>
    </dl>
    <p class="field-hint">密码修改接口暂未开放，后续可在这里接入当前密码校验与新密码保存。</p>
  </section>
</template>

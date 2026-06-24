<script setup lang="ts">
import { onMounted } from "vue";
import { RouterLink, useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import { useUserLeases } from "./useUserLeases";

const ui = useUiStore();
const auth = useAuthStore();
const router = useRouter();
const { activeLeases, activeSessions, loadLeases } = useUserLeases();

onMounted(() => {
  ui.clearMessages();
  if (!auth.isAuthenticated) {
    void router.replace("/login");
    return;
  }
  void loadLeases();
});
</script>

<template>
  <div class="dashboard-page">
    <section class="dashboard-welcome">
      <div>
        <h2>你好，{{ auth.user?.display_name ?? auth.user?.username }}</h2>
        <p class="public-page-subtitle">这里是你的开发板租赁工作台，可以快速查看账户、租赁和会话状态。</p>
      </div>
      <div class="dashboard-kpis">
        <div>
          <span>{{ activeLeases.length }}</span>
          <p>我的租赁</p>
        </div>
        <div>
          <span>{{ activeSessions.length }}</span>
          <p>当前会话</p>
        </div>
      </div>
    </section>

    <section class="dashboard-quick-grid">
      <RouterLink class="dashboard-quick-card card" to="/dashboard/account">
        <span class="form-section-icon info"><Icon name="user" :size="16" /></span>
        <strong>账户信息</strong>
        <span>查看个人资料、角色和密码修改入口。</span>
      </RouterLink>
      <RouterLink class="dashboard-quick-card card" to="/dashboard/leases">
        <span class="form-section-icon boot"><Icon name="clipboard" :size="16" /></span>
        <strong>我的租赁</strong>
        <span>查看已有租赁和当前租约会话。</span>
      </RouterLink>
      <RouterLink class="dashboard-quick-card card" to="/dashboard/sessions">
        <span class="form-section-icon info"><Icon name="terminal" :size="16" /></span>
        <strong>租约会话</strong>
        <span>查看当前已经启动的租赁会话。</span>
      </RouterLink>
      <RouterLink class="dashboard-quick-card card" to="/resources">
        <span class="form-section-icon info"><Icon name="cpu-board" :size="16" /></span>
        <strong>资源申请</strong>
        <span>前往资源页面选择开发板并申请租赁。</span>
      </RouterLink>
    </section>
  </div>
</template>

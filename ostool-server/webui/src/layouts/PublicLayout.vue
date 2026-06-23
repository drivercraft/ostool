<script setup lang="ts">
import { computed, watch } from "vue";
import { RouterLink, RouterView, useRoute } from "vue-router";

import AppDialog from "@/components/AppDialog.vue";
import Icon, { type IconName } from "@/components/Icon.vue";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";

const route = useRoute();
const ui = useUiStore();
const auth = useAuthStore();

watch(
  () => route.meta.title,
  (title) => {
    const fallback = (title as string | undefined) ?? "首页";
    ui.setTitle(fallback);
    document.title = `${fallback} - ostool 开发板租赁平台`;
  },
  { immediate: true },
);

const navItems = computed(() => [
  { to: "/", label: "首页", exact: true, icon: "home" as IconName },
  { to: "/resources", label: "资源", exact: false, icon: "cpu-board" as IconName },
  { to: "/docs", label: "文档", exact: false, icon: "book" as IconName },
]);

function isExactActive(item: { to: string; exact: boolean }) {
  return item.exact ? route.path === item.to : route.path.startsWith(item.to);
}
</script>

<template>
  <div class="public-shell">
    <header class="public-topbar">
      <div class="public-topbar-inner">
        <RouterLink class="brand-lockup" to="/">
          <span class="brand-mark"><Icon name="circuit" :size="22" /></span>
          <div>
            <strong>ostool</strong>
            <span>开发板租赁平台</span>
          </div>
        </RouterLink>
        <nav class="public-nav" aria-label="平台导航">
          <RouterLink
            v-for="item in navItems"
            :key="item.to"
            :to="item.to"
            class="public-nav-link"
            :class="{ 'is-active': isExactActive(item) }"
          >
            <Icon :name="item.icon" :size="16" class="nav-link-icon" />
            <span>{{ item.label }}</span>
          </RouterLink>
        </nav>
        <div class="public-actions">
          <template v-if="auth.isAuthenticated">
            <RouterLink
              v-if="auth.isAdmin"
              class="btn btn-ghost btn-sm"
              to="/admin/overview"
            >
              <Icon name="shield" :size="14" class="btn-icon" />
              管理台
            </RouterLink>
            <RouterLink class="public-user-chip" to="/dashboard">
              <span class="avatar-circle">
                {{ (auth.user?.display_name ?? auth.user?.username ?? "?").slice(0, 1).toUpperCase() }}
              </span>
              <span class="public-user-name">{{ auth.user?.display_name ?? auth.user?.username }}</span>
              <Icon name="chevron-right" :size="14" class="public-user-caret" />
            </RouterLink>
          </template>
          <template v-else>
            <RouterLink class="btn btn-ghost btn-sm" to="/login">
              <Icon name="login" :size="14" class="btn-icon" />
              登录
            </RouterLink>
            <RouterLink class="btn btn-primary btn-sm" to="/register">
              <Icon name="user" :size="14" class="btn-icon" />
              注册
            </RouterLink>
          </template>
        </div>
      </div>
    </header>

    <main class="public-main">
      <RouterView />
    </main>

    <footer class="public-footer">
      <div class="public-footer-inner">
        <div class="footer-brand">
          <span class="brand-mark"><Icon name="circuit" :size="18" /></span>
          <div>
            <strong>ostool</strong>
            <span>开发板租赁平台</span>
          </div>
        </div>
        <nav class="footer-nav">
          <RouterLink to="/">首页</RouterLink>
          <RouterLink to="/resources">可用资源</RouterLink>
          <RouterLink to="/docs">使用文档</RouterLink>
          <RouterLink to="/login">登录</RouterLink>
        </nav>
        <span class="footer-copy">© ostool 开发板租赁平台</span>
      </div>
    </footer>
    <AppDialog
      v-if="ui.dialog"
      :dialog="ui.dialog"
      @confirm="ui.closeDialog(true)"
      @cancel="ui.closeDialog(false)"
    />
  </div>
</template>

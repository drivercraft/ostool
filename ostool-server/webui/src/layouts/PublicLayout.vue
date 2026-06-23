<script setup lang="ts">
import { computed, watch } from "vue";
import { RouterLink, RouterView, useRoute } from "vue-router";

import Icon, { type IconName } from "@/components/Icon.vue";
import NoticeBanner from "@/components/NoticeBanner.vue";
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
            <RouterLink class="ghost-button compact-button" to="/dashboard">
              <Icon name="user" :size="14" class="btn-icon" />
              控制台
            </RouterLink>
          </template>
          <template v-else>
            <RouterLink class="ghost-button compact-button" to="/login">
              <Icon name="login" :size="14" class="btn-icon" />
              登录
            </RouterLink>
          </template>
          <RouterLink
            v-if="auth.isAdmin"
            class="primary-button compact-button"
            to="/admin/overview"
          >
            <Icon name="shield" :size="14" class="btn-icon" />
            管理台
          </RouterLink>
        </div>
      </div>
    </header>

    <main class="public-main">
      <div v-if="ui.successMessage || ui.errorMessage" class="public-notices">
        <NoticeBanner
          v-if="ui.successMessage"
          tone="success"
          :message="ui.successMessage"
        />
        <NoticeBanner
          v-if="ui.errorMessage"
          tone="error"
          :message="ui.errorMessage"
        />
      </div>
      <RouterView />
    </main>

    <footer class="public-footer">
      <div class="public-footer-inner">
        <span>© ostool 开发板租赁平台</span>
        <nav class="footer-nav">
          <RouterLink to="/docs">使用文档</RouterLink>
          <RouterLink to="/resources">可用资源</RouterLink>
        </nav>
      </div>
    </footer>
  </div>
</template>

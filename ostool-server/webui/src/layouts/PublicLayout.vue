<script setup lang="ts">
import { computed, watch } from "vue";
import { RouterLink, RouterView, useRoute } from "vue-router";

import AccountMenu from "@/components/AccountMenu.vue";
import AnnouncementBar from "@/components/AnnouncementBar.vue";
import AppDialog from "@/components/AppDialog.vue";
import AppFooter from "@/components/AppFooter.vue";
import Icon, { type IconName } from "@/components/Icon.vue";
import ThemeMenu from "@/components/ThemeMenu.vue";
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
  <div class="site-shell">
    <header class="site-header">
      <RouterLink class="brand-lockup" to="/">
        <span class="brand-mark"><Icon name="circuit" :size="22" /></span>
        <div>
          <strong>ostool</strong>
          <span>开发板租赁平台</span>
        </div>
      </RouterLink>
      <nav class="site-nav" aria-label="平台导航">
        <RouterLink
          v-for="item in navItems"
          :key="item.to"
          :to="item.to"
          class="site-nav-link"
          :class="{ 'is-active': isExactActive(item) }"
        >
          <Icon :name="item.icon" :size="16" class="nav-link-icon" />
          <span>{{ item.label }}</span>
        </RouterLink>
      </nav>
      <div class="site-actions">
        <ThemeMenu />
        <button class="btn-icon-only" type="button" title="语言" aria-label="语言">
          <Icon name="globe" :size="18" />
        </button>
        <template v-if="auth.isAuthenticated">
          <AccountMenu />
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
    </header>

    <AnnouncementBar />

    <main class="site-main">
      <RouterView />
    </main>

    <AppFooter />
    <AppDialog
      v-if="ui.dialog"
      :dialog="ui.dialog"
      @confirm="ui.closeDialog(true)"
      @cancel="ui.closeDialog(false)"
    />
  </div>
</template>

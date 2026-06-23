<script setup lang="ts">
import { computed, watch } from "vue";
import { RouterLink, RouterView, useRoute, useRouter } from "vue-router";

import AppDialog from "@/components/AppDialog.vue";
import Icon, { type IconName } from "@/components/Icon.vue";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";

const route = useRoute();
const router = useRouter();
const ui = useUiStore();
const auth = useAuthStore();

const workspaceNavItems = [
  { to: "/dashboard#overview", hash: "#overview", label: "工作台总览", icon: "chart" as IconName },
  { to: "/dashboard#request", hash: "#request", label: "申请会话", icon: "plus" as IconName },
  { to: "/dashboard#leases", hash: "#leases", label: "我的租赁", icon: "clipboard" as IconName },
  { to: "/dashboard#account", hash: "#account", label: "账号信息", icon: "user" as IconName },
];

const topNavItems = [
  { to: "/", label: "首页", icon: "home" as IconName, exact: true },
  { to: "/resources", label: "资源", icon: "cpu-board" as IconName, exact: false },
  { to: "/docs", label: "文档", icon: "book" as IconName, exact: false },
];

const displayName = computed(() => auth.user?.display_name ?? auth.user?.username ?? "用户");
const avatarInitial = computed(() => displayName.value.slice(0, 1).toUpperCase());

watch(
  () => route.meta.title,
  (title) => {
    const fallback = (title as string | undefined) ?? "用户控制台";
    ui.setTitle(fallback);
    document.title = `${fallback} - ostool 开发板租赁平台`;
  },
  { immediate: true },
);

async function logout() {
  await auth.logoutUser();
  void router.push("/");
}

function isWorkspaceActive(item: { hash: string }) {
  if (route.path !== "/dashboard") {
    return false;
  }
  if (item.hash === "#overview") {
    return route.hash === "" || route.hash === item.hash;
  }
  return route.hash === item.hash;
}

function isTopActive(item: { to: string; exact: boolean }) {
  return item.exact ? route.path === item.to : route.path.startsWith(item.to);
}

function scrollToWorkspaceSection(hash: string) {
  window.requestAnimationFrame(() => {
    document.querySelector(hash)?.scrollIntoView({
      behavior: "smooth",
      block: "start",
    });
  });
}
</script>

<template>
  <div class="user-shell">
    <header class="public-topbar user-topbar">
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
            v-for="item in topNavItems"
            :key="item.to"
            class="public-nav-link"
            :class="{ 'is-active': isTopActive(item) }"
            :to="item.to"
          >
            <Icon :name="item.icon" :size="16" class="nav-link-icon" />
            <span>{{ item.label }}</span>
          </RouterLink>
        </nav>

        <div class="public-actions">
          <RouterLink v-if="auth.isAdmin" class="btn btn-ghost btn-sm" to="/admin/overview">
            <Icon name="shield" :size="14" class="btn-icon" />
            管理台
          </RouterLink>
          <button class="btn btn-ghost btn-sm" type="button" @click="logout">
            <Icon name="logout" :size="14" class="btn-icon" />
            退出登录
          </button>
        </div>
      </div>
    </header>

    <main class="public-main user-main">
      <div class="user-app-shell">
        <aside class="user-sidebar">
          <div class="user-sidebar-profile">
            <span class="avatar-circle">{{ avatarInitial }}</span>
            <div>
              <strong>{{ displayName }}</strong>
              <span>{{ auth.isAdmin ? "管理员" : "普通用户" }}</span>
            </div>
          </div>

          <nav class="user-nav" aria-label="用户导航">
            <RouterLink
              v-for="item in workspaceNavItems"
              :key="item.to"
              class="user-nav-link"
              :class="{ 'is-active': isWorkspaceActive(item) }"
              :to="item.to"
              @click="scrollToWorkspaceSection(item.hash)"
            >
              <Icon :name="item.icon" :size="17" class="nav-link-icon" />
              <span>{{ item.label }}</span>
            </RouterLink>
          </nav>
        </aside>

        <section class="user-content-shell">
          <RouterView />
        </section>
      </div>
    </main>
    <AppDialog
      v-if="ui.dialog"
      :dialog="ui.dialog"
      @confirm="ui.closeDialog(true)"
      @cancel="ui.closeDialog(false)"
    />
  </div>
</template>

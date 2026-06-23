<script setup lang="ts">
import { watch } from "vue";
import { RouterLink, RouterView, useRoute, useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import NoticeBanner from "@/components/NoticeBanner.vue";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";

const route = useRoute();
const router = useRouter();
const ui = useUiStore();
const auth = useAuthStore();

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
</script>

<template>
  <div class="user-shell">
    <header class="public-topbar user-topbar">
      <div class="public-topbar-inner">
        <RouterLink class="brand-lockup" to="/">
          <span class="brand-mark"><Icon name="circuit" :size="22" /></span>
          <div>
            <strong>ostool</strong>
            <span>用户控制台</span>
          </div>
        </RouterLink>

        <nav class="public-nav" aria-label="用户导航">
          <RouterLink class="public-nav-link" to="/dashboard">
            <Icon name="clipboard" :size="16" class="nav-link-icon" />
            <span>我的租赁</span>
          </RouterLink>
          <RouterLink class="public-nav-link" to="/resources">
            <Icon name="cpu-board" :size="16" class="nav-link-icon" />
            <span>资源</span>
          </RouterLink>
          <RouterLink class="public-nav-link" to="/docs">
            <Icon name="book" :size="16" class="nav-link-icon" />
            <span>文档</span>
          </RouterLink>
        </nav>

        <div class="public-actions">
          <RouterLink
            v-if="auth.isAdmin"
            class="ghost-button compact-button"
            to="/admin/overview"
          >
            <Icon name="shield" :size="14" class="btn-icon" />
            管理台
          </RouterLink>
          <button class="primary-button compact-button" type="button" @click="logout">
            <Icon name="logout" :size="14" class="btn-icon" />
            退出登录
          </button>
        </div>
      </div>
    </header>

    <main class="public-main user-main">
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
  </div>
</template>

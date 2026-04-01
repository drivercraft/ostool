<script setup lang="ts">
import { computed, watch } from "vue";
import { RouterLink, RouterView, useRoute } from "vue-router";

import NoticeBanner from "@/components/NoticeBanner.vue";
import { useUiStore } from "@/stores/ui";

const route = useRoute();
const ui = useUiStore();

watch(
  () => route.meta.title,
  (title) => {
    ui.setTitle((title as string | undefined) ?? "管理台");
    document.title = `${ui.title} - ostool-server`;
  },
  { immediate: true },
);

const navItems = computed(() => [
  { to: "/overview", label: "总览" },
  { to: "/boards", label: "开发板" },
  { to: "/dtbs", label: "DTB 管理" },
  { to: "/sessions", label: "会话租约" },
  { to: "/tftp", label: "TFTP 配置" },
  { to: "/server", label: "Server 配置" },
]);
</script>

<template>
  <div class="app-shell">
    <aside class="sidebar">
      <div class="brand">
        <span class="brand-mark">OS</span>
        <div>
          <h1>ostool-server</h1>
          <p>开发板管理台</p>
        </div>
      </div>
      <nav class="nav-list">
        <RouterLink
          v-for="item in navItems"
          :key="item.to"
          :to="item.to"
          class="nav-link"
          active-class="is-active"
        >
          {{ item.label }}
        </RouterLink>
      </nav>
    </aside>
    <div class="app-content">
      <header class="topbar">
        <div>
          <p class="eyebrow">控制面板</p>
          <h2>{{ ui.title }}</h2>
        </div>
      </header>

      <main class="page-body">
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
        <RouterView />
      </main>
    </div>
  </div>
</template>

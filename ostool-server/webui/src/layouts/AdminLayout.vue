<script setup lang="ts">
import { computed, reactive, watch } from "vue";
import { RouterLink, RouterView, useRoute } from "vue-router";

import AccountMenu from "@/components/AccountMenu.vue";
import AppDialog from "@/components/AppDialog.vue";
import Icon, { type IconName } from "@/components/Icon.vue";
import ThemeMenu from "@/components/ThemeMenu.vue";
import { useUiStore } from "@/stores/ui";

const route = useRoute();
const ui = useUiStore();

watch(
  () => route.meta.title,
  (title) => {
    ui.setTitle((title as string | undefined) ?? "管理台");
    document.title = `${ui.title} - ostool 平台`;
  },
  { immediate: true },
);

interface NavItem {
  to: string;
  label: string;
}

interface NavGroupItem {
  type: "group";
  id: string;
  label: string;
  icon: IconName;
  items: NavItem[];
}

interface NavLinkItem {
  type: "link";
  to: string;
  label: string;
  icon: IconName;
}

type AdminNavItem = NavGroupItem | NavLinkItem;

const navItems = computed<AdminNavItem[]>(() => [
  {
    type: "group",
    id: "overview",
    label: "概览",
    icon: "chart",
    items: [{ to: "/admin/overview", label: "运行总览" }],
  },
  {
    type: "group",
    id: "resources",
    label: "资源管理",
    icon: "cpu-board",
    items: [
      { to: "/admin/resources/boards", label: "开发板配置" },
      { to: "/admin/resources/dtbs", label: "DTB 配置" },
      { to: "/admin/resources/tftp", label: "TFTP 配置" },
    ],
  },
  {
    type: "group",
    id: "rentals",
    label: "租赁管理",
    icon: "clipboard",
    items: [
      { to: "/admin/rentals/leases", label: "租赁情况" },
      { to: "/admin/rentals/sessions", label: "会话租约" },
    ],
  },
  {
    type: "group",
    id: "users",
    label: "用户管理",
    icon: "users",
    items: [
      { to: "/admin/users/list", label: "用户列表" },
      { to: "/admin/users/roles", label: "角色与权限" },
    ],
  },
  {
    type: "link",
    to: "/admin/audit",
    label: "审计日志",
    icon: "shield",
  },
]);

const collapsedGroups = reactive<Record<string, boolean>>({});

function isNavItemActive(to: string) {
  return route.path === to || route.path.startsWith(`${to}/`);
}

function isGroupActive(group: NavGroupItem) {
  return group.items.some((item) => isNavItemActive(item.to));
}

function isGroupCollapsed(group: NavGroupItem) {
  return collapsedGroups[group.id] ?? false;
}

function toggleGroup(group: NavGroupItem) {
  collapsedGroups[group.id] = !isGroupCollapsed(group);
}
</script>

<template>
  <div class="app-shell">
    <aside class="sidebar">
      <div class="brand">
        <span class="brand-mark"><Icon name="circuit" :size="22" /></span>
        <h1>ostool-server</h1>
      </div>
      <nav class="nav-list" aria-label="管理导航">
        <template v-for="item in navItems" :key="item.label">
          <RouterLink
            v-if="item.type === 'link'"
            :to="item.to"
            class="nav-group-trigger nav-standalone-link"
            :class="{ 'is-active': isNavItemActive(item.to) }"
          >
            <Icon :name="item.icon" :size="16" class="nav-link-icon" />
            <span>{{ item.label }}</span>
          </RouterLink>
          <section v-else class="nav-group">
            <button
              class="nav-group-trigger"
              :class="{ 'is-active': isGroupActive(item) }"
              type="button"
              :aria-expanded="!isGroupCollapsed(item)"
              @click="toggleGroup(item)"
            >
              <Icon :name="item.icon" :size="16" class="nav-link-icon" />
              <span>{{ item.label }}</span>
              <Icon
                name="chevron-right"
                :size="15"
                class="nav-group-chevron"
                :class="{ 'is-open': !isGroupCollapsed(item) }"
              />
            </button>
            <div v-show="!isGroupCollapsed(item)" class="nav-sub-list">
              <RouterLink
                v-for="child in item.items"
                :key="child.to"
                :to="child.to"
                class="nav-link"
                :class="{ 'is-active': isNavItemActive(child.to) }"
              >
                <span class="nav-sub-marker" aria-hidden="true"></span>
                <span>{{ child.label }}</span>
              </RouterLink>
            </div>
          </section>
        </template>
      </nav>
      <div class="sidebar-footer">
        <RouterLink
          class="nav-group-trigger nav-group-bottom-link"
          :class="{ 'is-active': isNavItemActive('/admin/settings/server') }"
          to="/admin/settings/server"
        >
          <Icon name="settings" :size="16" class="nav-link-icon" />
          <span>系统设置</span>
        </RouterLink>
      </div>
    </aside>
    <div class="app-content">
      <header class="topbar">
        <div class="topbar-title">
          <h2>{{ ui.title }}</h2>
        </div>
        <div class="topbar-actions" aria-label="管理台工具栏">
          <button class="btn-icon-only" type="button" title="消息">
            <Icon name="bell" :size="18" />
          </button>
          <ThemeMenu />
          <button class="btn-icon-only" type="button" title="语言" aria-label="语言">
            <Icon name="globe" :size="18" />
          </button>
          <AccountMenu />
        </div>
      </header>

      <main class="page-body">
        <RouterView />
      </main>
    </div>
    <AppDialog
      v-if="ui.dialog"
      :dialog="ui.dialog"
      @confirm="ui.closeDialog(true)"
      @cancel="ui.closeDialog(false)"
    />
  </div>
</template>

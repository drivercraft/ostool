<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { RouterLink, useRouter } from "vue-router";

import Icon, { type IconName } from "@/components/Icon.vue";
import { useAuthStore } from "@/stores/auth";

const auth = useAuthStore();
const router = useRouter();
const open = ref(false);

interface AccountMenuItem {
  to: string;
  label: string;
  icon: IconName;
}

const displayName = computed(() => auth.user?.display_name || auth.user?.username || "用户");
const avatarText = computed(() => displayName.value.slice(0, 1).toUpperCase());
const roleLabel = computed(() => (auth.isAdmin ? "管理员" : "注册用户"));

const menuItems = computed<AccountMenuItem[]>(() => {
  const items: AccountMenuItem[] = [
    { to: "/dashboard", label: "工作台总览", icon: "chart" },
    { to: "/dashboard/account", label: "用户信息", icon: "user" },
    { to: "/dashboard/leases", label: "我的租赁", icon: "clipboard" },
    { to: "/dashboard/sessions", label: "我的会话", icon: "terminal" },
    { to: "/dashboard/issues", label: "问题反馈", icon: "bell" },
    { to: "/leases/new", label: "申请租赁", icon: "plus" },
  ];
  if (auth.isAdmin) {
    items.unshift({ to: "/admin/overview", label: "管理台", icon: "shield" });
  }
  return items;
});

function close() {
  open.value = false;
}

async function logout() {
  close();
  await auth.logoutUser();
  void router.push("/");
}

function closeOnDocumentClick(event: MouseEvent) {
  if ((event.target as HTMLElement | null)?.closest(".account-menu-shell")) {
    return;
  }
  close();
}

function closeOnEscape(event: KeyboardEvent) {
  if (event.key === "Escape") {
    close();
  }
}

onMounted(() => {
  document.addEventListener("click", closeOnDocumentClick);
  document.addEventListener("keydown", closeOnEscape);
});

onUnmounted(() => {
  document.removeEventListener("click", closeOnDocumentClick);
  document.removeEventListener("keydown", closeOnEscape);
});
</script>

<template>
  <div class="account-menu-shell">
    <button
      class="account-avatar-button"
      type="button"
      :title="displayName"
      :aria-expanded="open"
      aria-label="账户菜单"
      @click="open = !open"
    >
      {{ avatarText }}
    </button>

    <div v-if="open" class="account-menu">
      <div class="account-menu-profile">
        <span class="account-menu-avatar">{{ avatarText }}</span>
        <strong>{{ displayName }}</strong>
        <span>{{ roleLabel }}</span>
      </div>

      <nav class="account-menu-list" aria-label="账户导航">
        <RouterLink
          v-for="item in menuItems"
          :key="item.to"
          class="account-menu-item"
          :to="item.to"
          @click="close"
        >
          <Icon :name="item.icon" :size="15" />
          <span>{{ item.label }}</span>
        </RouterLink>
      </nav>

      <div class="account-menu-divider"></div>
      <button class="account-menu-item account-menu-item-danger" type="button" @click="logout">
        <Icon name="logout" :size="15" />
        <span>退出登录</span>
      </button>
    </div>
  </div>
</template>

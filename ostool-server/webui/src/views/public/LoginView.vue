<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";

import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";

const ui = useUiStore();
const auth = useAuthStore();
const router = useRouter();
const route = useRoute();

const username = ref("");
const password = ref("");
const asAdmin = ref(false);
const submitting = ref(false);

const requestedAdmin = computed(() => route.query.mode === "admin");
const nextPath = computed(() => {
  const next = route.query.next;
  return typeof next === "string" && next.startsWith("/") ? next : null;
});

async function submit() {
  if (submitting.value) {
    return;
  }
  if (!username.value.trim() || !password.value) {
    ui.setError("请输入用户名和密码");
    return;
  }
  submitting.value = true;
  const adminMode = asAdmin.value || requestedAdmin.value;
  try {
    await auth.login(username.value.trim(), password.value);
  } catch (error) {
    submitting.value = false;
    ui.setError((error as Error).message);
    return;
  }
  if (adminMode && !auth.isAdmin) {
    submitting.value = false;
    await auth.logoutUser();
    ui.setError("该账号没有管理员权限");
    return;
  }
  ui.setSuccess(adminMode ? "已进入管理员会话" : "登录成功");
  const fallback = adminMode ? "/admin/overview" : "/dashboard";
  void router.push(nextPath.value ?? fallback);
}

onMounted(() => {
  ui.clearMessages();
  if (requestedAdmin.value) {
    asAdmin.value = true;
  }
});
</script>

<template>
  <div class="page-body public-page-body">
    <div class="login-shell">
      <section class="login-card panel">
        <header class="login-header">
          <p class="eyebrow">登录平台</p>
          <h2>欢迎回到 ostool</h2>
          <p class="login-subtitle">
            登录后即可申请会话、上传镜像并使用远程串口终端。
          </p>
        </header>

        <form class="login-form" @submit.prevent="submit">
          <label class="field">
            <span>用户名</span>
            <input
              v-model="username"
              autocomplete="username"
              placeholder="例如：demo"
              :disabled="submitting"
            />
          </label>
          <label class="field">
            <span>密码</span>
            <input
              v-model="password"
              type="password"
              autocomplete="current-password"
              placeholder="例如：demo"
              :disabled="submitting"
            />
          </label>
          <label class="checkbox-field">
            <input v-model="asAdmin" type="checkbox" :disabled="submitting" />
            <span>以管理员身份登录（进入管理台）</span>
          </label>
          <div class="toolbar-actions">
            <button
              class="primary-button"
              type="submit"
              :disabled="submitting"
            >
              {{ submitting ? "登录中..." : "登录" }}
            </button>
          </div>
        </form>

        <div class="login-footer">
          还没有账号？
          <RouterLink class="inline-link" to="/resources">先浏览资源</RouterLink>
        </div>
      </section>
    </div>
  </div>
</template>

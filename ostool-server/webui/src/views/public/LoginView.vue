<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import NoticeBanner from "@/components/NoticeBanner.vue";
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
  <div class="auth-shell">
    <aside class="auth-aside">
      <RouterLink class="auth-aside-brand" to="/">
        <span class="brand-mark"><Icon name="circuit" :size="20" /></span>
        ostool
      </RouterLink>
      <div class="auth-aside-copy">
        <h2>把硬件实验室<br />变成可调度的共享资源</h2>
        <p>统一的开发板资源池、远程串口与网络启动，让团队把精力聚焦在系统与镜像本身。</p>
        <ul class="auth-aside-points">
          <li><Icon name="check" :size="16" /> 按需申请，自动分配空闲开发板</li>
          <li><Icon name="check" :size="16" /> WebSocket 远程串口终端</li>
          <li><Icon name="check" :size="16" /> TFTP / UEFI HTTP Boot 一键启动</li>
        </ul>
      </div>
      <p class="auth-aside-quote">© ostool 开发板租赁平台</p>
    </aside>

    <main class="auth-panel">
      <section class="auth-card">
        <header class="auth-header">
          <p class="eyebrow">登录平台</p>
          <h2>欢迎回到 ostool</h2>
          <p>登录后即可申请会话、上传镜像并使用远程串口终端。</p>
        </header>

        <NoticeBanner
          v-if="ui.errorMessage"
          tone="error"
          :message="ui.errorMessage"
          class="auth-notice"
        />

        <form class="auth-form" @submit.prevent="submit">
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
          <button class="primary-button" type="submit" :disabled="submitting">
            {{ submitting ? "登录中..." : "登录" }}
            <Icon v-if="!submitting" name="arrow-right" :size="16" class="btn-icon" />
          </button>
        </form>

        <div class="auth-divider">或</div>
        <div class="auth-footer">
          还没有账号？<RouterLink class="inline-link" to="/register">立即注册</RouterLink>
        </div>
      </section>
    </main>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import { api } from "@/api";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import type { CaptchaResponse } from "@/types/api";

const ui = useUiStore();
const auth = useAuthStore();
const router = useRouter();
const route = useRoute();

const username = ref("");
const password = ref("");
const captchaAnswer = ref("");
const captcha = ref<CaptchaResponse | null>(null);
const captchaLoading = ref(false);
const submitting = ref(false);

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
  if (!captcha.value || !captchaAnswer.value.trim()) {
    ui.setError("请输入验证码");
    return;
  }
  submitting.value = true;
  try {
    await auth.login(
      username.value.trim(),
      password.value,
      captcha.value.token,
      captchaAnswer.value.trim(),
    );
  } catch (error) {
    submitting.value = false;
    captchaAnswer.value = "";
    void loadCaptcha();
    ui.setError((error as Error).message);
    return;
  }
  if (nextPath.value?.startsWith("/admin") && !auth.isAdmin) {
    ui.setError("当前账号无权访问管理台");
    void router.push("/dashboard");
    return;
  }
  ui.setSuccess("登录成功");
  const fallback = auth.isAdmin ? "/admin/overview" : "/dashboard";
  void router.push(nextPath.value ?? fallback);
}

async function loadCaptcha() {
  captchaLoading.value = true;
  try {
    captcha.value = await api.getCaptcha();
    captchaAnswer.value = "";
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    captchaLoading.value = false;
  }
}

onMounted(() => {
  ui.clearMessages();
  void loadCaptcha();
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
          <h2>欢迎回到 ostool</h2>
          <p>登录后即可申请会话、上传镜像并使用远程串口终端。</p>
        </header>

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
          <div class="captcha-row">
            <label class="field captcha-input">
              <span>验证码</span>
              <input
                v-model="captchaAnswer"
                autocomplete="off"
                inputmode="text"
                placeholder="输入右侧验证码"
                :disabled="submitting || captchaLoading"
              />
            </label>
            <button
              class="captcha-image"
              type="button"
              :disabled="submitting || captchaLoading"
              title="刷新验证码"
              @click="loadCaptcha"
            >
              <span v-if="captchaLoading">加载中</span>
              <span v-else-if="captcha" v-html="captcha.image_svg"></span>
              <span v-else>刷新</span>
            </button>
          </div>
          <button class="btn btn-primary" type="submit" :disabled="submitting">
            {{ submitting ? "登录中..." : "登录" }}
            <Icon v-if="!submitting" name="arrow-right" :size="16" class="btn-icon" />
          </button>
        </form>

        <div class="auth-divider">或</div>
        <div class="auth-footer">
          还没有账号？<RouterLink class="inline-link" to="/register">立即注册</RouterLink>
        </div>
        <p class="auth-legal">
          登录即表示你已阅读并同意
          <RouterLink class="inline-link" to="/terms">用户协议</RouterLink>
          和
          <RouterLink class="inline-link" to="/privacy">隐私政策</RouterLink>
        </p>
      </section>
    </main>
  </div>
</template>

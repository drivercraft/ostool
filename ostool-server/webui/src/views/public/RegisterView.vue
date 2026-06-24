<script setup lang="ts">
import { computed, ref } from "vue";
import { onMounted } from "vue";
import { RouterLink } from "vue-router";

import Icon from "@/components/Icon.vue";
import { api } from "@/api";
import { USERNAME_PATTERN, VALIDATION_LIMITS } from "@/constants/validation";
import { useUiStore } from "@/stores/ui";
import type { CaptchaResponse } from "@/types/api";

const ui = useUiStore();

const username = ref("");
const displayName = ref("");
const email = ref("");
const password = ref("");
const confirmPassword = ref("");
const agreed = ref(false);
const captchaAnswer = ref("");
const captcha = ref<CaptchaResponse | null>(null);
const captchaLoading = ref(false);
const submitting = ref(false);

const passwordsMismatch = computed(
  () => confirmPassword.value.length > 0 && password.value !== confirmPassword.value,
);

function submit() {
  if (submitting.value) {
    return;
  }
  if (
    !username.value.trim() ||
    !email.value.trim() ||
    !password.value ||
    !displayName.value.trim()
  ) {
    ui.setError("请完整填写用户名、姓名、邮箱与密码");
    return;
  }
  if (password.value !== confirmPassword.value) {
    ui.setError("两次输入的密码不一致");
    return;
  }
  if (!captcha.value || !captchaAnswer.value.trim()) {
    ui.setError("请输入验证码");
    return;
  }
  if (!agreed.value) {
    ui.setError("请先阅读并同意平台使用条款");
    return;
  }
  submitting.value = true;
  // 当前平台账号由管理员统一开通；自助注册通道暂未开放。
  // 在此给出明确反馈，避免调用不存在的后端接口。
  window.setTimeout(() => {
    submitting.value = false;
    captchaAnswer.value = "";
    void loadCaptcha();
    ui.setSuccess("注册申请已记录。当前账号由平台管理员统一开通，请联平台管理员完成激活。");
  }, 400);
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
        <h2>加入共享开发板实验室</h2>
        <p>注册账号后即可在浏览器中申请开发板、上传镜像并连接远程串口，开启高效的远程调试流程。</p>
        <ul class="auth-aside-points">
          <li><Icon name="sparkles" :size="16" /> 统一资源池，按板型与标签筛选</li>
          <li><Icon name="terminal" :size="16" /> 远程串口，免去插拔 USB 线</li>
          <li><Icon name="shield" :size="16" /> 租约与心跳，资源自动回收</li>
        </ul>
      </div>
      <p class="auth-aside-quote">© ostool 开发板租赁平台</p>
    </aside>

    <main class="auth-panel">
      <section class="auth-card">
        <header class="auth-header">
          <h2>注册 ostool 账号</h2>
          <p>填写下方信息提交注册申请，账号由平台管理员审核开通。</p>
        </header>

        <form class="auth-form" @submit.prevent="submit">
          <label class="field is-required">
            <span>用户名</span>
            <input
              v-model="username"
              autocomplete="username"
              :minlength="VALIDATION_LIMITS.usernameMin"
              :maxlength="VALIDATION_LIMITS.usernameMax"
              :pattern="USERNAME_PATTERN"
              placeholder="登录用账号，建议小写字母、数字或 -/_"
              :disabled="submitting"
            />
          </label>
          <label class="field is-required">
            <span>姓名 / 显示名</span>
            <input
              v-model="displayName"
              :minlength="VALIDATION_LIMITS.displayNameMin"
              :maxlength="VALIDATION_LIMITS.displayNameMax"
              placeholder="用于页面展示，例如：张三"
              :disabled="submitting"
            />
          </label>
          <label class="field is-required">
            <span>邮箱</span>
            <input
              v-model="email"
              type="email"
              autocomplete="email"
              :minlength="VALIDATION_LIMITS.emailMin"
              :maxlength="VALIDATION_LIMITS.emailMax"
              placeholder="用于联系和账号通知，例如 you@example.com"
              :disabled="submitting"
            />
          </label>
          <label class="field is-required">
            <span>密码</span>
            <input
              v-model="password"
              type="password"
              autocomplete="new-password"
              :minlength="VALIDATION_LIMITS.passwordMin"
              :maxlength="VALIDATION_LIMITS.passwordMax"
              placeholder="必填，建议至少 8 位并包含字母和数字"
              :disabled="submitting"
            />
          </label>
          <label class="field is-required">
            <span>确认密码</span>
            <input
              v-model="confirmPassword"
              type="password"
              autocomplete="new-password"
              :minlength="VALIDATION_LIMITS.passwordMin"
              :maxlength="VALIDATION_LIMITS.passwordMax"
              placeholder="再次输入相同密码"
              :disabled="submitting"
            />
          </label>
          <p v-if="passwordsMismatch" class="auth-hint">两次输入的密码不一致</p>
          <div class="captcha-row">
            <label class="field captcha-input is-required">
              <span>验证码</span>
              <input
                v-model="captchaAnswer"
                autocomplete="off"
                inputmode="text"
                :maxlength="VALIDATION_LIMITS.captchaMax"
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
          <label class="checkbox-field">
            <input v-model="agreed" type="checkbox" :disabled="submitting" />
            <span>
              我已阅读并同意
              <RouterLink class="inline-link" to="/terms">用户协议</RouterLink>
              和
              <RouterLink class="inline-link" to="/privacy">隐私政策</RouterLink>
            </span>
          </label>
          <button class="btn btn-primary" type="submit" :disabled="submitting">
            {{ submitting ? "提交中..." : "提交注册申请" }}
            <Icon v-if="!submitting" name="arrow-right" :size="16" class="btn-icon" />
          </button>
        </form>

        <div class="auth-divider">或</div>
        <div class="auth-footer">
          已有账号？<RouterLink class="inline-link" to="/login">直接登录</RouterLink>
        </div>
      </section>
    </main>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RouterLink, useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import { api } from "@/api";
import { USERNAME_PATTERN, VALIDATION_LIMITS } from "@/constants/validation";
import { useUiStore } from "@/stores/ui";
import type { CaptchaResponse, RegistrationPolicyResponse } from "@/types/api";

const ui = useUiStore();
const router = useRouter();

const username = ref("");
const displayName = ref("");
const email = ref("");
const password = ref("");
const confirmPassword = ref("");
const phone = ref("");
const department = ref("");
const title = ref("");
const agreed = ref(false);
const captchaAnswer = ref("");
const captcha = ref<CaptchaResponse | null>(null);
const captchaLoading = ref(false);
const submitting = ref(false);

// Reflects server-side `registration_mode`. Loaded on mount; when `closed`,
// the form is hidden and a notice tells the visitor registration is disabled.
const policy = ref<RegistrationPolicyResponse | null>(null);
const policyLoading = ref(true);

const registrationClosed = computed(() => policy.value?.mode === "closed");

const passwordsMismatch = computed(
  () => confirmPassword.value.length > 0 && password.value !== confirmPassword.value,
);

async function loadPolicy() {
  policyLoading.value = true;
  try {
    policy.value = await api.auth.getRegistrationPolicy();
  } catch (error) {
    // If the endpoint is unreachable, default to closed so we never show a
    // form that cannot succeed.
    policy.value = { mode: "closed", self_service_enabled: false };
    ui.setError((error as Error).message);
  } finally {
    policyLoading.value = false;
  }
}

async function submit() {
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
  try {
    const result = await api.auth.register({
      username: username.value.trim(),
      display_name: displayName.value.trim(),
      email: email.value.trim(),
      password: password.value,
      confirm_password: confirmPassword.value,
      captcha_token: captcha.value.token,
      captcha_answer: captchaAnswer.value.trim(),
      phone: phone.value.trim() || undefined,
      department: department.value.trim() || undefined,
      title: title.value.trim() || undefined,
    });
    captchaAnswer.value = "";
    if (result.outcome === "closed") {
      ui.setError("当前平台已关闭自助注册，请联系管理员开通账号。");
    } else if (result.outcome === "pending") {
      ui.setSuccess(
        `注册申请已提交，${result.display_name}。账号正在等待管理员审核，审核通过后即可登录。`,
      );
      void router.push({ name: "login" });
    } else {
      ui.setSuccess(`注册成功，${result.display_name}。现在可以使用账号登录。`);
      void router.push({
        name: "login",
        query: { username: result.username },
      });
    }
  } catch (error) {
    ui.setError((error as Error).message);
    void loadCaptcha();
  } finally {
    submitting.value = false;
  }
}

async function loadCaptcha() {
  captchaLoading.value = true;
  try {
    captcha.value = await api.auth.getCaptcha();
    captchaAnswer.value = "";
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    captchaLoading.value = false;
  }
}

onMounted(async () => {
  await loadPolicy();
  if (!registrationClosed.value) {
    void loadCaptcha();
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
          <p v-if="policy && policy.mode === 'approval'">
            填写下方信息提交注册申请，提交后账号进入待审核状态，由平台管理员审核通过后即可登录。
          </p>
          <p v-else-if="policy && policy.mode === 'auto'">
            填写下方信息完成注册，注册成功后即可使用账号登录平台。
          </p>
          <p v-else>填写下方信息提交注册申请，账号由平台管理员审核开通。</p>
        </header>

        <div v-if="policyLoading" class="empty-state">正在加载注册设置...</div>
        <div v-else-if="registrationClosed" class="empty-state">
          <div class="empty-state-icon">&#9888;</div>
          当前平台已关闭自助注册。请联系管理员开通账号后再登录。
          <RouterLink class="btn btn-ghost btn-sm" to="/login">前往登录</RouterLink>
        </div>

        <form v-else class="auth-form" @submit.prevent="submit">
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
          <div class="auth-form-row">
            <label class="field">
              <span>手机号（选填）</span>
              <input
                v-model="phone"
                autocomplete="tel"
                :maxlength="VALIDATION_LIMITS.phoneMax"
                placeholder="便于紧急联系，可留空"
                :disabled="submitting"
              />
            </label>
            <label class="field">
              <span>部门（选填）</span>
              <input
                v-model="department"
                :maxlength="VALIDATION_LIMITS.departmentMax"
                placeholder="例如：内核组"
                :disabled="submitting"
              />
            </label>
          </div>
          <label class="field">
            <span>职位（选填）</span>
            <input
              v-model="title"
              :maxlength="VALIDATION_LIMITS.titleMax"
              placeholder="例如：嵌入式工程师"
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

        <template v-if="!registrationClosed">
          <div class="auth-divider">或</div>
          <div class="auth-footer">
            已有账号？<RouterLink class="inline-link" to="/login">直接登录</RouterLink>
          </div>
        </template>
      </section>
    </main>
  </div>
</template>

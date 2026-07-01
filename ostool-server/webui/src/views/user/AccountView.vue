<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";

import { api } from "@/api";
import Icon from "@/components/Icon.vue";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import { formatLeaseTime } from "@/composables/useUserLeases";

const ui = useUiStore();
const auth = useAuthStore();
const router = useRouter();
const submittingPassword = ref(false);
const passwordForm = ref({
  current_password: "",
  new_password: "",
  confirm_new_password: "",
});

const accountFields = computed(() => [
  ["用户名", auth.user?.username ?? "-"],
  ["显示名称", auth.user?.display_name ?? "-"],
  ["邮箱", auth.user?.email ?? "-"],
  ["手机号", auth.user?.phone ?? "-"],
  ["部门", auth.user?.department ?? "-"],
  ["职位", auth.user?.title ?? "-"],
  ["最后登录", auth.user?.last_login_at ? formatLeaseTime(auth.user.last_login_at) : "-"],
  ["角色", auth.user?.roles.map((role) => role.display_name || role.name).join("、") || "普通用户"],
]);
const passwordMismatch = computed(() =>
  passwordForm.value.confirm_new_password.length > 0
    && passwordForm.value.new_password !== passwordForm.value.confirm_new_password,
);
const passwordReady = computed(() =>
  passwordForm.value.current_password.length >= 8
    && passwordForm.value.new_password.length >= 8
    && passwordForm.value.confirm_new_password.length >= 8
    && !passwordMismatch.value,
);

async function submitPasswordChange() {
  if (!passwordReady.value) {
    ui.setError("请输入两次一致且不少于 8 位的新密码");
    return;
  }
  submittingPassword.value = true;
  try {
    await api.user.updateUserPassword({
      current_password: passwordForm.value.current_password,
      new_password: passwordForm.value.new_password,
      confirm_new_password: passwordForm.value.confirm_new_password,
    });
    passwordForm.value.current_password = "";
    passwordForm.value.new_password = "";
    passwordForm.value.confirm_new_password = "";
    ui.setSuccess("密码已修改");
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    submittingPassword.value = false;
  }
}

onMounted(() => {
  ui.clearMessages();
  if (!auth.isAuthenticated) {
    void router.replace("/login");
  }
});
</script>

<template>
  <section class="dashboard-account-card card">
    <div class="panel-heading compact dashboard-section-heading">
      <div>
        <h3>账户信息</h3>
        <p class="muted">当前登录账户的资料与安全设置。</p>
      </div>
    </div>

    <section class="dashboard-account-section">
      <div class="form-section-header">
        <span class="form-section-icon info"><Icon name="user" :size="16" /></span>
        <h4>基本资料</h4>
      </div>
      <dl class="profile-dl dashboard-account-dl">
        <div v-for="[label, value] in accountFields" :key="label">
          <dt>{{ label }}</dt>
          <dd>{{ value }}</dd>
        </div>
      </dl>
    </section>

    <section class="dashboard-account-section">
      <div class="form-section-header">
        <span class="form-section-icon boot"><Icon name="key" :size="16" /></span>
        <h4>账号安全</h4>
      </div>
      <form class="dashboard-password-form" @submit.prevent="submitPasswordChange">
        <label class="field is-required form-grid-wide">
          <span>当前密码</span>
          <input
            v-model="passwordForm.current_password"
            type="password"
            minlength="8"
            maxlength="128"
            autocomplete="current-password"
            :disabled="submittingPassword"
            placeholder="请输入当前登录密码"
          />
        </label>
        <label class="field is-required">
          <span>新密码</span>
          <input
            v-model="passwordForm.new_password"
            type="password"
            minlength="8"
            maxlength="128"
            autocomplete="new-password"
            :disabled="submittingPassword"
            placeholder="请输入不少于 8 位的新密码"
          />
        </label>
        <label class="field is-required">
          <span>确认新密码</span>
          <input
            v-model="passwordForm.confirm_new_password"
            type="password"
            minlength="8"
            maxlength="128"
            autocomplete="new-password"
            :disabled="submittingPassword"
            placeholder="请再次输入新密码"
          />
        </label>
        <p v-if="passwordMismatch" class="field-error form-grid-wide">两次输入的新密码不一致。</p>
        <div class="dashboard-form-actions form-grid-wide">
          <button class="btn btn-primary" type="submit" :disabled="!passwordReady || submittingPassword">
            <Icon name="key" :size="14" class="btn-icon" />
            {{ submittingPassword ? "修改中..." : "修改密码" }}
          </button>
        </div>
      </form>
    </section>
  </section>
</template>

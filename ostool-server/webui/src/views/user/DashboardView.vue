<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RouterLink, useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import { api } from "@/api";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import type { LeaseResponse } from "@/types/api";

const ui = useUiStore();
const auth = useAuthStore();
const router = useRouter();

const leases = ref<LeaseResponse[]>([]);
const loading = ref(true);

const activeLeases = computed(() =>
  leases.value.filter((item) => item.lease.state === "active"),
);
const activeSessions = computed(() =>
  leases.value.filter((item) => item.lease.state === "active" && item.session),
);
const accountFields = computed(() => [
  ["用户名", auth.user?.username ?? "-"],
  ["显示名称", auth.user?.display_name ?? "-"],
  ["邮箱", auth.user?.email ?? "-"],
  ["手机号", auth.user?.phone ?? "-"],
  ["部门", auth.user?.department ?? "-"],
  ["职位", auth.user?.title ?? "-"],
  ["最后登录", auth.user?.last_login_at ? formatTime(auth.user.last_login_at) : "-"],
  ["角色", auth.user?.roles.map((role) => role.display_name || role.name).join("、") || "普通用户"],
]);

function formatTime(iso: string): string {
  const parsed = Date.parse(iso);
  if (!Number.isFinite(parsed)) {
    return iso;
  }
  return new Date(parsed).toLocaleString();
}

function remainingLabel(iso: string): string {
  const parsed = Date.parse(iso);
  if (!Number.isFinite(parsed)) {
    return "-";
  }
  const remaining = parsed - Date.now();
  if (remaining <= 0) {
    return "已过期";
  }
  const minutes = Math.floor(remaining / 60000);
  if (minutes < 60) {
    return `${minutes} 分钟`;
  }
  const hours = Math.floor(minutes / 60);
  return `${hours} 小时 ${minutes % 60} 分钟`;
}

async function loadAll() {
  loading.value = true;
  try {
    const leaseList = await api.listUserLeases();
    leases.value = leaseList.leases;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

async function releaseSession(leaseId: string) {
  const confirmed = await ui.confirm({
    tone: "danger",
    title: "释放租赁",
    message: `确认释放租赁 ${leaseId}？相关开发板将归还到资源池。`,
    confirmLabel: "释放",
  });
  if (!confirmed) {
    return;
  }
  try {
    await api.deleteLease(leaseId);
    ui.setSuccess(`已释放租赁 ${leaseId}`);
    await loadAll();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

onMounted(() => {
  ui.clearMessages();
  if (!auth.isAuthenticated) {
    void router.replace("/login");
    return;
  }
  void loadAll();
});
</script>

<template>
  <div class="dashboard-page">
    <section id="overview" class="dashboard-welcome">
      <div>
        <h2>你好，{{ auth.user?.display_name ?? auth.user?.username }}</h2>
        <p class="public-page-subtitle">查看账户资料、当前租赁情况和正在使用的租约会话。</p>
      </div>
      <div class="dashboard-kpis">
        <div>
          <span>{{ activeLeases.length }}</span>
          <p>我的租赁</p>
        </div>
        <div>
          <span>{{ activeSessions.length }}</span>
          <p>当前会话</p>
        </div>
      </div>
    </section>

    <section id="account" class="dashboard-account-card card">
      <div class="panel-heading compact dashboard-section-heading">
        <div>
          <h3>账户信息</h3>
          <p class="muted">当前登录账户的基础资料。</p>
        </div>
        <button class="btn btn-ghost btn-sm" type="button" disabled>
          <Icon name="key" :size="14" class="btn-icon" />
          修改密码
        </button>
      </div>
      <dl class="profile-dl dashboard-account-dl">
        <div v-for="[label, value] in accountFields" :key="label">
          <dt>{{ label }}</dt>
          <dd>{{ value }}</dd>
        </div>
      </dl>
      <p class="field-hint">密码修改接口暂未开放，后续可在这里接入当前密码校验与新密码保存。</p>
    </section>

    <section id="leases" class="dashboard-rentals-section card">
      <div class="panel-heading compact dashboard-section-heading">
        <div>
          <h3>我的租赁</h3>
          <p class="muted">包含自己已有的租赁，以及当前租约会话。</p>
        </div>
        <div class="dashboard-form-actions">
          <RouterLink class="btn btn-primary btn-sm" to="/dashboard/leases/new">
            <Icon name="plus" :size="14" class="btn-icon" />
            新增租赁
          </RouterLink>
          <button class="btn btn-ghost btn-sm" :disabled="loading" @click="loadAll">刷新</button>
        </div>
      </div>

      <section class="dashboard-subsection">
        <div class="dashboard-subsection-head">
          <h4>租赁情况</h4>
          <span>{{ activeLeases.length }} 条活跃</span>
        </div>
        <div v-if="loading" class="empty-state"><div class="empty-state-icon">&#9641;</div>正在加载租赁...</div>
        <div v-else-if="activeLeases.length === 0" class="empty-state">
          <div class="empty-state-icon">&#9641;</div>
          当前没有活跃租赁。点击“新增租赁”申请一块开发板。
        </div>

        <div v-else class="board-card-grid">
          <article v-for="item in activeLeases" :key="item.lease.id" class="board-card">
            <div class="board-card-header">
              <div class="board-card-id">
                <strong>{{ item.lease.board_id }}</strong>
                <span class="board-card-meta">{{ item.lease.board_type }} · {{ item.lease.state }}</span>
              </div>
              <span class="pill pill-success">活跃</span>
            </div>

            <dl class="key-value-list lease-card-stats">
              <div><dt>开始时间</dt><dd>{{ formatTime(item.lease.starts_at) }}</dd></div>
              <div><dt>结束时间</dt><dd>{{ formatTime(item.lease.expires_at) }}</dd></div>
              <div><dt>剩余时长</dt><dd :style="{color: remainingLabel(item.lease.expires_at) === '已过期' ? 'var(--c-danger)' : 'var(--c-success)'}">{{ remainingLabel(item.lease.expires_at) }}</dd></div>
            </dl>

            <div v-if="item.lease.required_tags.length > 0" class="lease-tags">
              <span>标签:</span>
              <span v-for="tag in item.lease.required_tags" :key="tag" class="tag">{{ tag }}</span>
            </div>

            <div class="toolbar-actions">
              <button class="btn btn-danger btn-sm" type="button" @click="releaseSession(item.lease.id)">释放租赁</button>
            </div>
          </article>
        </div>
      </section>

      <section class="dashboard-subsection">
        <div class="dashboard-subsection-head">
          <h4>租约会话</h4>
          <span>{{ activeSessions.length }} 个当前会话</span>
        </div>
        <div v-if="loading" class="empty-state"><div class="empty-state-icon">&#9641;</div>正在加载会话...</div>
        <div v-else-if="activeSessions.length === 0" class="empty-state">
          <div class="empty-state-icon">&#9641;</div>
          当前没有正在运行的租约会话。
        </div>

        <div v-else class="board-card-grid">
          <article v-for="session in activeSessions" :key="session.lease.id" class="board-card">
          <div class="board-card-header">
            <div class="board-card-id">
              <strong>{{ session.lease.board_id }}</strong>
              <span class="board-card-meta">{{ session.lease.board_type }} &middot; {{ session.session?.state ?? session.lease.state }}</span>
            </div>
            <span class="pill pill-success">活跃</span>
          </div>

          <dl class="key-value-list lease-card-stats">
            <div><dt>租赁 ID</dt><dd><code>{{ session.lease.id }}</code></dd></div>
            <div><dt>会话 ID</dt><dd><code>{{ session.lease.session_id || "未启用" }}</code></dd></div>
            <div><dt>到期时间</dt><dd>{{ formatTime(session.lease.expires_at) }}</dd></div>
            <div><dt>剩余时长</dt><dd :style="{color: remainingLabel(session.lease.expires_at) === '已过期' ? 'var(--c-danger)' : 'var(--c-success)'}">{{ remainingLabel(session.lease.expires_at) }}</dd></div>
          </dl>

          <div v-if="session.lease.required_tags.length > 0" class="lease-tags">
            <span>标签:</span>
            <span v-for="tag in session.lease.required_tags" :key="tag" class="tag">{{ tag }}</span>
          </div>

          <div class="toolbar-actions">
            <button class="btn btn-danger btn-sm" type="button" @click="releaseSession(session.lease.id)">释放租赁</button>
          </div>
          </article>
        </div>
      </section>
    </section>
  </div>
</template>

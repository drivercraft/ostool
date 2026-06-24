<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import { api } from "@/api";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import type {
  BoardTypeSummary,
  LeaseResponse,
} from "@/types/api";

const ui = useUiStore();
const auth = useAuthStore();
const router = useRouter();

const leases = ref<LeaseResponse[]>([]);
const boardTypes = ref<BoardTypeSummary[]>([]);
const loading = ref(true);
const submitting = ref(false);

const selectedBoardType = ref("");
const requiredTags = ref("");

const activeSessions = computed(() =>
  leases.value.filter((item) => item.lease.state === "active" && item.session),
);

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
    const [types, leaseList] = await Promise.all([
      api.listBoardTypes(),
      api.listUserLeases(),
    ]);
    boardTypes.value = types;
    leases.value = leaseList.leases;
    if (selectedBoardType.value === "" && boardTypes.value.length > 0) {
      const candidate = boardTypes.value.find((item) => item.available > 0);
      selectedBoardType.value = candidate?.board_type ?? boardTypes.value[0].board_type;
    }
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

async function createSession() {
  if (!auth.user) {
    return;
  }
  if (submitting.value) {
    return;
  }
  if (!selectedBoardType.value) {
    ui.setError("请先选择要申请的开发板型号");
    return;
  }
  submitting.value = true;
  try {
    const tags = requiredTags.value
      .split(",")
      .map((tag) => tag.trim())
      .filter((tag) => tag.length > 0);
    const created = await api.createLease({
      board_type: selectedBoardType.value,
      required_tags: tags,
    });
    requiredTags.value = "";
    ui.setSuccess(`已创建租赁 ${created.lease.id}，开发板 ${created.lease.board_id} 已分配`);
    await loadAll();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    submitting.value = false;
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
  const params = new URLSearchParams(window.location.search);
  const presetBoardType = params.get("board_type");
  if (presetBoardType) {
    selectedBoardType.value = presetBoardType;
  }
  void loadAll();
});
</script>

<template>
  <div class="dashboard-page">
    <section id="overview" class="dashboard-welcome">
      <div>
        <h2>你好，{{ auth.user?.display_name ?? auth.user?.username }}</h2>
        <p class="public-page-subtitle">在这里申请开发板会话、查看分配资源并释放不再使用的租约。</p>
      </div>
      <div class="dashboard-kpis">
        <div>
          <span>{{ activeSessions.length }}</span>
          <p>活跃租约</p>
        </div>
        <div>
          <span>{{ boardTypes.length }}</span>
          <p>可选型号</p>
        </div>
      </div>
    </section>

    <section id="request" class="dashboard-request-panel card">
      <div class="panel-heading compact"><h3>申请新会话</h3></div>
      <div class="dashboard-form dashboard-form-inline">
        <label class="form-group">
          <span class="form-label">开发板型号</span>
          <select v-model="selectedBoardType" :disabled="loading || submitting" class="input">
            <option value="" disabled>请选择...</option>
            <option v-for="board in boardTypes" :key="board.board_type" :value="board.board_type">
              {{ board.board_type }}（可用 {{ board.available }} / {{ board.total }}）
            </option>
          </select>
        </label>
        <label class="form-group">
          <span class="form-label">必需标签（逗号分隔，可留空）</span>
          <input v-model="requiredTags" placeholder="例如：lab, usb" :disabled="submitting" class="input" />
        </label>
        <div class="dashboard-form-actions">
          <button class="btn btn-primary" :disabled="submitting || loading" @click="createSession">
            {{ submitting ? '申请中...' : '申请会话' }}
          </button>
          <button class="btn btn-ghost btn-sm" :disabled="loading" @click="loadAll">刷新数据</button>
        </div>
      </div>
    </section>

    <section id="leases" class="dashboard-session-section">
      <div class="panel-heading">
        <div>
          <h3>当前持有的开发板租约 <span>{{ activeSessions.length }} 个活跃</span></h3>
        </div>
      </div>

      <div v-if="loading" class="empty-state"><div class="empty-state-icon">&#9641;</div>正在加载会话...</div>
      <div v-else-if="activeSessions.length === 0" class="empty-state">
        <div class="empty-state-icon">&#9641;</div>
        当前没有活跃会话。选择型号后点击"申请会话"即可获得一块开发板。
      </div>

      <div v-else class="board-card-grid">
        <article v-for="session in activeSessions" :key="session.lease.id" class="board-card">
          <div class="board-card-header">
            <div class="board-card-id">
              <code>{{ session.lease.board_id }}</code>
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

    <section id="account" class="dashboard-account-card card">
      <div class="panel-heading compact"><h3>账号信息</h3></div>
      <dl class="profile-dl dashboard-account-dl">
        <div><dt>用户名</dt><dd><code>{{ auth.user?.username }}</code></dd></div>
        <div><dt>邮箱</dt><dd>{{ auth.user?.email }}</dd></div>
        <div><dt>用户 ID</dt><dd><code>{{ auth.user?.id }}</code></dd></div>
        <div><dt>角色</dt><dd>{{ auth.isAdmin ? '管理员' : '普通用户' }}</dd></div>
      </dl>
    </section>
  </div>
</template>

<script setup lang="ts">
import { onMounted } from "vue";
import { RouterLink, useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import { getSessionDisplayStatus } from "@/utils/sessionStatus";
import { formatLeaseTime, remainingLeaseLabel, useUserLeases } from "@/composables/useUserLeases";

const ui = useUiStore();
const auth = useAuthStore();
const router = useRouter();
const {
  loading,
  activeSessions,
  loadLeases,
  releaseLease,
} = useUserLeases();

onMounted(() => {
  ui.clearMessages();
  if (!auth.isAuthenticated) {
    void router.replace("/login");
    return;
  }
  void loadLeases();
});
</script>

<template>
  <section class="dashboard-rentals-section card">
    <div class="panel-heading compact dashboard-section-heading panel-heading--actions-only">
      <div class="dashboard-form-actions">
        <RouterLink class="btn btn-ghost btn-sm" to="/dashboard/leases">
          <Icon name="clipboard" :size="14" class="btn-icon" />
          返回租赁
        </RouterLink>
        <button class="btn btn-ghost btn-sm" :disabled="loading" @click="loadLeases">刷新</button>
      </div>
    </div>

    <section class="dashboard-subsection">
      <div class="dashboard-subsection-head">
        <h4>当前会话</h4>
        <span>{{ activeSessions.length }} 个当前会话</span>
      </div>
      <div v-if="loading" class="empty-state"><div class="empty-state-icon">&#9641;</div>正在加载会话...</div>
      <div v-else-if="activeSessions.length === 0" class="empty-state">
        <div class="empty-state-icon">&#9641;</div>
        暂无会话数据
      </div>

      <div v-else class="board-card-grid">
        <article v-for="session in activeSessions" :key="session.lease.id" class="board-card lease-card">
          <div class="board-card-header">
            <div class="board-card-id">
              <strong>{{ session.lease.board_id }}</strong>
              <span class="board-card-meta">{{ session.lease.board_type }}</span>
            </div>
            <StatusPill
              :tone="getSessionDisplayStatus(session.session?.state).tone"
              :label="getSessionDisplayStatus(session.session?.state).label"
            />
          </div>

          <div class="lease-card-window">
            <span>会话租期</span>
            <strong>{{ formatLeaseTime(session.lease.starts_at) }}</strong>
            <small>至 {{ formatLeaseTime(session.lease.expires_at) }}</small>
          </div>

          <dl class="key-value-list lease-card-stats">
            <div><dt>租赁 ID</dt><dd>{{ session.lease.id }}</dd></div>
            <div><dt>会话 ID</dt><dd>{{ session.lease.session_id || "未启用" }}</dd></div>
            <div><dt>剩余时长</dt><dd :style="{color: remainingLeaseLabel(session.lease.expires_at) === '已过期' ? 'var(--c-danger)' : 'var(--c-success)'}">{{ remainingLeaseLabel(session.lease.expires_at) }}</dd></div>
          </dl>

          <div v-if="session.lease.required_tags.length > 0" class="lease-tags">
            <span>标签:</span>
            <span v-for="tag in session.lease.required_tags" :key="tag" class="tag">{{ tag }}</span>
          </div>

          <div class="toolbar-actions">
            <button class="btn btn-danger btn-sm" type="button" @click="releaseLease(session.lease.id)">释放租赁</button>
          </div>
        </article>
      </div>
    </section>
  </section>
</template>

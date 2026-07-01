<script setup lang="ts">
import { onMounted, ref } from "vue";

import Icon, { type IconName } from "@/components/Icon.vue";
import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { AdminOverviewResponse } from "@/types/api";
import { describeTftpStatus } from "@/utils/tftpStatus";

const ui = useUiStore();
const loading = ref(true);
const overview = ref<AdminOverviewResponse | null>(null);

async function loadOverview() {
  loading.value = true;
  try {
    overview.value = await api.admin.getOverview();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  ui.clearMessages();
  void loadOverview();
});
</script>

<template>
  <div class="space-y-6">
    <!-- Header -->
    <div class="flex items-center justify-between">
      <div>
        <h2 class="text-xl font-bold">运行状态</h2>
        <p class="text-sm muted" style="margin-top: .15rem">平台当前资源与健康度总览</p>
      </div>
    </div>

    <div v-if="loading" class="card loading-block">
      <div class="spinner" />
      <p>正在加载总览信息...</p>
    </div>

    <template v-else-if="overview">
      <!-- Metric Cards -->
      <div class="stat-grid">
        <div class="stat-card">
          <span class="stat-icon"><Icon name="cpu-board" :size="18" /></span>
          <div class="stat-label">开发板总数</div>
          <div class="stat-value">{{ overview.board_count_total }}</div>
          <div class="stat-hint">当前平台纳管资源总量</div>
        </div>
        <div class="stat-card">
          <span class="stat-icon" style="color: var(--c-success); background: var(--c-success-bg)"><Icon name="check" :size="18" /></span>
          <div class="stat-label">可用开发板</div>
          <div class="stat-value" style="color: var(--c-success)">{{ overview.board_count_available }}</div>
          <div class="stat-hint">可立即分配的空闲资源</div>
        </div>
        <div class="stat-card">
          <span class="stat-icon" style="color: var(--c-violet); background: var(--c-violet-soft)"><Icon name="pulse" :size="18" /></span>
          <div class="stat-label">活跃会话</div>
          <div class="stat-value" style="color: var(--c-brand)">{{ overview.active_session_count }}</div>
          <div class="stat-hint">正在使用中的租赁会话</div>
        </div>
        <div class="stat-card">
          <span class="stat-icon" :style="{ color: overview.tftp_status.healthy ? 'var(--c-success)' : 'var(--c-warning)', background: overview.tftp_status.healthy ? 'var(--c-success-bg)' : 'var(--c-warning-bg)' }"><Icon name="shield" :size="18" /></span>
          <div class="stat-label">运行健康度</div>
          <div class="stat-value" :style="{ color: overview.tftp_status.healthy ? 'var(--c-success)' : 'var(--c-warning)' }">
            {{ overview.tftp_status.healthy ? '98%' : '62%' }}
          </div>
          <div class="stat-hint">TFTP {{ describeTftpStatus(overview.tftp_status).label }}</div>
        </div>
      </div>

      <!-- Board Type Chart -->
      <div class="card">
        <div class="panel-heading compact">
          <h3>开发板型号排行</h3>
          <span class="pill pill-neutral">Top {{ Math.min(8, overview.board_types.length) }}</span>
        </div>
        <div class="space-y-3">
          <div
            v-for="item in overview.board_types.slice(0, 8)"
            :key="item.board_type"
            class="flex items-center gap-3"
          >
            <span class="text-sm font-semibold truncate" style="width: 140px">{{ item.board_type }}</span>
            <div class="progress flex-1">
              <span
                :style="{ width: `${Math.max(8, Math.min(100, (item.total / (overview.board_types[0]?.total || 1)) * 100))}%` }"
              />
            </div>
            <span class="text-sm font-mono muted" style="width: 64px" >{{ item.available }}/{{ item.total }}</span>
          </div>
          <div v-if="overview.board_types.length === 0" class="empty-state">
            暂无开发板型号数据。
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

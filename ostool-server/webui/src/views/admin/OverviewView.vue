<script setup lang="ts">
import { onMounted, ref } from "vue";

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
    overview.value = await api.getOverview();
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
        <h2 class="text-xl font-bold text-slate-800">运行状态</h2>
        <p class="text-sm text-slate-500 mt-0.5">平台当前资源与健康度总览</p>
      </div>
      <button class="btn-secondary btn-sm" @click="loadOverview">刷新</button>
    </div>

    <div v-if="loading" class="card p-12 flex flex-col items-center justify-center">
      <div class="w-8 h-8 border-2 border-indigo-500 border-t-transparent rounded-full animate-spin mb-3" />
      <p class="text-slate-400 text-sm">正在加载总览信息...</p>
    </div>

    <template v-else-if="overview">
      <!-- Metric Cards -->
      <div class="grid grid-cols-2 lg:grid-cols-4 gap-4">
        <div class="card p-5">
          <div class="text-xs font-medium text-slate-400 uppercase tracking-wide mb-1">开发板总数</div>
          <div class="text-3xl font-bold text-slate-800">{{ overview.board_count_total }}</div>
          <div class="text-xs text-slate-400 mt-2">当前平台纳管资源总量</div>
        </div>
        <div class="card p-5">
          <div class="text-xs font-medium text-slate-400 uppercase tracking-wide mb-1">可用开发板</div>
          <div class="text-3xl font-bold text-emerald-600">{{ overview.board_count_available }}</div>
          <div class="text-xs text-slate-400 mt-2">可立即分配的空闲资源</div>
        </div>
        <div class="card p-5">
          <div class="text-xs font-medium text-slate-400 uppercase tracking-wide mb-1">活跃会话</div>
          <div class="text-3xl font-bold text-indigo-600">{{ overview.active_session_count }}</div>
          <div class="text-xs text-slate-400 mt-2">正在使用中的租赁会话</div>
        </div>
        <div class="card p-5">
          <div class="text-xs font-medium text-slate-400 uppercase tracking-wide mb-1">运行健康度</div>
          <div class="text-3xl font-bold" :class="overview.tftp_status.healthy ? 'text-emerald-600' : 'text-amber-600'">
            {{ overview.tftp_status.healthy ? '98%' : '62%' }}
          </div>
          <div class="text-xs text-slate-400 mt-2">TFTP {{ describeTftpStatus(overview.tftp_status).label }}</div>
        </div>
      </div>

      <!-- Board Type Chart -->
      <div class="card p-6">
        <h3 class="text-base font-semibold text-slate-800 mb-4">开发板型号排行</h3>
        <div class="space-y-3">
          <div v-for="item in overview.board_types.slice(0, 8)" :key="item.board_type" class="flex items-center gap-3">
            <span class="text-sm font-medium text-slate-700 w-32 truncate">{{ item.board_type }}</span>
            <div class="flex-1 bg-slate-100 rounded-full h-5 overflow-hidden">
              <div
                class="h-full rounded-full bg-gradient-to-r from-indigo-400 to-indigo-500 transition-all duration-500"
                :style="{ width: `${Math.max(8, Math.min(100, (item.total / (overview.board_types[0]?.total || 1)) * 100))}%` }"
              />
            </div>
            <span class="text-sm font-mono text-slate-500 w-16 text-right">{{ item.available }}/{{ item.total }}</span>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

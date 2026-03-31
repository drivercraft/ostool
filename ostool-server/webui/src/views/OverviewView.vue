<script setup lang="ts">
import { onMounted, ref } from "vue";

import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api/client";
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
  <section class="page-grid">
    <div class="panel panel-hero">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">服务概况</p>
          <h3>开发板池与 TFTP 当前状态</h3>
        </div>
        <button class="ghost-button" @click="loadOverview">刷新</button>
      </div>

      <div v-if="loading" class="empty-state">正在加载总览信息...</div>
      <template v-else-if="overview">
        <div class="stats-grid">
          <article class="stat-card">
            <span class="stat-label">开发板总数</span>
            <strong>{{ overview.board_count_total }}</strong>
          </article>
          <article class="stat-card">
            <span class="stat-label">可用开发板</span>
            <strong>{{ overview.board_count_available }}</strong>
          </article>
          <article class="stat-card">
            <span class="stat-label">禁用开发板</span>
            <strong>{{ overview.disabled_board_count }}</strong>
          </article>
          <article class="stat-card">
            <span class="stat-label">活跃会话</span>
            <strong>{{ overview.active_session_count }}</strong>
          </article>
        </div>

        <div class="split-grid">
          <div class="panel nested-panel">
            <div class="panel-heading compact">
              <h4>按类型统计</h4>
            </div>
            <table class="data-table">
              <thead>
                <tr>
                  <th>类型</th>
                  <th>标签</th>
                  <th>总数</th>
                  <th>可用</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="item in overview.board_types" :key="item.board_type">
                  <td>{{ item.board_type }}</td>
                  <td>{{ item.tags.join(", ") || "-" }}</td>
                  <td>{{ item.total }}</td>
                  <td>{{ item.available }}</td>
                </tr>
              </tbody>
            </table>
          </div>

          <div class="panel nested-panel">
            <div class="panel-heading compact">
              <h4>TFTP 诊断</h4>
              <StatusPill
                :tone="describeTftpStatus(overview.tftp_status).tone"
                :label="describeTftpStatus(overview.tftp_status).label"
              />
            </div>
            <dl class="key-value-list">
              <div>
                <dt>Provider</dt>
                <dd>{{ overview.tftp_status.provider }}</dd>
              </div>
              <div>
                <dt>根目录</dt>
                <dd>{{ overview.tftp_status.root_dir }}</dd>
              </div>
              <div>
                <dt>监听</dt>
                <dd>{{ overview.tftp_status.bind_addr_or_address || "-" }}</dd>
              </div>
              <div>
                <dt>写入状态</dt>
                <dd>{{ overview.tftp_status.writable ? "可写" : "不可写" }}</dd>
              </div>
            </dl>
            <p v-if="overview.tftp_status.last_error" class="diagnostic-error">
              {{ overview.tftp_status.last_error }}
            </p>
          </div>
        </div>
      </template>
    </div>
  </section>
</template>

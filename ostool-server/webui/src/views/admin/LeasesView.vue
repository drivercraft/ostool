<script setup lang="ts">
import { onMounted, ref } from "vue";

import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { LeaseResponse } from "@/types/api";

const ui = useUiStore();
const leases = ref<LeaseResponse[]>([]);
const loading = ref(true);

function leaseTone(state: string) {
  if (state === "active") {
    return "good";
  }
  if (state === "failed") {
    return "danger";
  }
  return "neutral";
}

async function loadLeases() {
  loading.value = true;
  try {
    leases.value = (await api.listAdminLeases()).leases;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

async function releaseLease(leaseId: string) {
  if (!window.confirm(`确认释放租赁 ${leaseId}？`)) {
    return;
  }
  try {
    await api.deleteAdminLease(leaseId);
    ui.setSuccess(`已发起释放租赁 ${leaseId}`);
    await loadLeases();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

onMounted(() => {
  void loadLeases();
});
</script>

<template>
  <section class="panel">
    <div class="panel-heading">
      <div>
        <p class="eyebrow">租赁管理</p>
        <h3>平台租赁记录</h3>
      </div>
      <button class="ghost-button compact-button" @click="loadLeases">刷新</button>
    </div>

    <div v-if="loading" class="empty-state">正在加载租赁...</div>
    <div v-else-if="leases.length === 0" class="empty-state">暂无租赁记录。</div>
    <div v-else class="table-scroll">
      <table class="data-table">
        <thead>
          <tr>
            <th>租赁 ID</th>
            <th>用户 ID</th>
            <th>开发板</th>
            <th>型号</th>
            <th>会话</th>
            <th>状态</th>
            <th>到期</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="item in leases" :key="item.lease.id">
            <td><code>{{ item.lease.id }}</code></td>
            <td><code>{{ item.lease.user_id }}</code></td>
            <td><code>{{ item.lease.board_id }}</code></td>
            <td>{{ item.lease.board_type }}</td>
            <td><code>{{ item.lease.session_id }}</code></td>
            <td>
              <StatusPill :tone="leaseTone(item.lease.state)" :label="item.lease.state" />
            </td>
            <td>{{ new Date(item.lease.expires_at).toLocaleString() }}</td>
            <td>
              <button
                class="danger-button compact-button"
                :disabled="item.lease.state !== 'active'"
                @click="releaseLease(item.lease.id)"
              >
                释放
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>

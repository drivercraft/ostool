<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { LeaseResponse } from "@/types/api";

const ui = useUiStore();
const leases = ref<LeaseResponse[]>([]);
const loading = ref(true);
const search = ref("");
const stateFilter = ref<"all" | "active" | "releasing" | "released" | "expired" | "failed">("all");

const filteredLeases = computed(() =>
  leases.value.filter((item) => {
    if (stateFilter.value !== "all" && item.lease.state !== stateFilter.value) {
      return false;
    }
    const query = search.value.trim().toLowerCase();
    if (!query) {
      return true;
    }
    return [
      item.lease.id,
      item.lease.user_id,
      item.lease.board_id,
      item.lease.board_type,
      item.lease.session_id,
      item.session?.client_name ?? "",
    ].some((value) => value.toLowerCase().includes(query));
  }),
);

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
  const confirmed = await ui.confirm({
    tone: "danger",
    title: "释放租赁",
    message: `确认释放租赁 ${leaseId}？`,
    confirmLabel: "释放",
  });
  if (!confirmed) {
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
  <section class="page-grid admin-list-page">
    <div class="panel admin-table-panel">
      <div class="admin-toolbar">
        <div class="admin-toolbar-left">
          <button class="btn btn-ghost btn-sm" @click="loadLeases">刷新</button>
        </div>
        <div class="admin-toolbar-right">
          <label class="search-field">
            <Icon name="search" :size="16" />
            <input v-model="search" placeholder="搜索租赁 / 用户 / 开发板" />
          </label>
          <label class="field filter-field">
            <span>状态</span>
            <select v-model="stateFilter">
              <option value="all">全部状态</option>
              <option value="active">active</option>
              <option value="releasing">releasing</option>
              <option value="released">released</option>
              <option value="expired">expired</option>
              <option value="failed">failed</option>
            </select>
          </label>
        </div>
      </div>

      <div v-if="loading" class="empty-state">正在加载租赁...</div>
      <div v-else-if="leases.length === 0" class="empty-state">暂无租赁记录。</div>
      <div v-else-if="filteredLeases.length === 0" class="empty-state">没有符合条件的租赁记录。</div>
      <div v-else class="table-scroll">
        <table class="data-table">
          <thead>
            <tr>
              <th class="col-index">序号</th>
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
            <tr v-for="(item, index) in filteredLeases" :key="item.lease.id">
              <td class="col-index">{{ index + 1 }}</td>
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
                  class="btn btn-danger btn-sm"
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

      <div v-if="!loading" class="table-statusbar">
        <span>{{ filteredLeases.length === 0 ? "暂无分页" : "第 1 / 共 1 页" }}</span>
        <span>本页 {{ filteredLeases.length }} 条</span>
        <span>筛选后 {{ filteredLeases.length }} 条 / 共 {{ leases.length }} 条</span>
      </div>
    </div>
  </section>
</template>

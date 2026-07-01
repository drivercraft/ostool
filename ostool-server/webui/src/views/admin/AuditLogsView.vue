<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";

import Icon from "@/components/Icon.vue";
import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { AdminAuditLogResponse } from "@/types/api";

const ui = useUiStore();
const loading = ref(true);
const logs = ref<AdminAuditLogResponse[]>([]);
const actionFilter = ref("");
const targetTypeFilter = ref("");
const operatorSearch = ref("");
const expandedId = ref<string | null>(null);
const page = ref(1);
const pageSize = 20;

const actionLabels: Record<string, string> = {
  "user.approve": "通过注册申请",
  "user.reject": "拒绝注册申请",
  "dtb.create": "上传 DTB",
  "dtb.rename": "重命名 DTB",
  "dtb.update": "替换 DTB",
  "dtb.update_metadata": "编辑 DTB 元数据",
  "dtb.delete": "删除 DTB",
};

const targetTypeLabels: Record<string, string> = {
  user: "用户",
  dtb_file: "DTB 文件",
};

const outcomeLabels: Record<string, string> = {
  success: "成功",
  failed: "失败",
  error: "错误",
};

const actionOptions = computed(() => (
  Array.from(new Set(logs.value.map((item) => item.action))).sort()
));

const targetTypeOptions = computed(() => (
  Array.from(new Set(logs.value.map((item) => item.target_type))).sort()
));

const filteredLogs = computed(() => {
  const operator = operatorSearch.value.trim().toLowerCase();
  return logs.value.filter((item) => {
    if (actionFilter.value && item.action !== actionFilter.value) {
      return false;
    }
    if (targetTypeFilter.value && item.target_type !== targetTypeFilter.value) {
      return false;
    }
    if (!operator) {
      return true;
    }
    return [
      item.actor_username ?? "",
      item.actor_user_id ?? "",
      item.ip_address ?? "",
      item.request_id ?? "",
    ].some((value) => value.toLowerCase().includes(operator));
  });
});

const totalPages = computed(() => Math.max(1, Math.ceil(filteredLogs.value.length / pageSize)));

const pagedLogs = computed(() => {
  const start = (page.value - 1) * pageSize;
  return filteredLogs.value.slice(start, start + pageSize);
});

function labelAction(action: string) {
  return actionLabels[action] ?? action;
}

function labelTargetType(type: string) {
  return targetTypeLabels[type] ?? type;
}

function labelOutcome(outcome: string) {
  return outcomeLabels[outcome] ?? outcome;
}

function operatorLabel(item: AdminAuditLogResponse) {
  return item.actor_username || item.actor_user_id || "系统";
}

function metadataText(value: unknown) {
  if (value == null) {
    return "-";
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function toggleExpanded(id: string) {
  expandedId.value = expandedId.value === id ? null : id;
}

function resetFilters() {
  actionFilter.value = "";
  targetTypeFilter.value = "";
  operatorSearch.value = "";
}

function prevPage() {
  page.value = Math.max(1, page.value - 1);
}

function nextPage() {
  page.value = Math.min(totalPages.value, page.value + 1);
}

async function loadAuditLogs() {
  loading.value = true;
  try {
    logs.value = (await api.admin.listAuditLogs()).logs;
    page.value = 1;
    expandedId.value = null;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

watch([actionFilter, targetTypeFilter, operatorSearch], () => {
  page.value = 1;
});

watch(totalPages, (value) => {
  if (page.value > value) {
    page.value = value;
  }
});

onMounted(() => {
  ui.clearMessages();
  void loadAuditLogs();
});
</script>

<template>
  <section class="page-grid admin-list-page audit-page">
    <div class="panel admin-table-panel">
      <div class="admin-toolbar">
        <div class="admin-toolbar-left">
          <button class="btn btn-ghost btn-sm" type="button" @click="resetFilters">
            清空
          </button>
          <button class="btn btn-ghost btn-sm" type="button" @click="loadAuditLogs">
            <Icon name="refresh" :size="15" class="btn-icon" />
            刷新
          </button>
        </div>
        <div class="admin-toolbar-right">
          <label class="search-field">
            <Icon name="search" :size="16" />
            <input
              v-model="operatorSearch"
              type="search"
              maxlength="128"
              placeholder="搜索操作人 / IP / 请求 ID"
            />
          </label>
          <label class="field select-field filter-field">
            <span>操作</span>
            <select v-model="actionFilter">
              <option value="">全部操作</option>
              <option v-for="action in actionOptions" :key="action" :value="action">
                {{ labelAction(action) }}
              </option>
            </select>
          </label>
          <label class="field select-field filter-field">
            <span>对象</span>
            <select v-model="targetTypeFilter">
              <option value="">全部对象</option>
              <option v-for="type in targetTypeOptions" :key="type" :value="type">
                {{ labelTargetType(type) }}
              </option>
            </select>
          </label>
        </div>
      </div>

      <div v-if="loading" class="empty-state">正在加载审计日志...</div>
      <div v-else class="table-scroll">
        <table class="data-table">
          <thead>
            <tr>
              <th class="col-index">序号</th>
              <th>时间</th>
              <th>操作</th>
              <th>对象</th>
              <th>结果</th>
              <th>操作人</th>
              <th>IP</th>
              <th>浏览器</th>
            </tr>
          </thead>
          <tbody>
            <template v-for="(item, index) in pagedLogs" :key="item.id">
              <tr class="audit-row" @click="toggleExpanded(item.id)">
                <td class="col-index">{{ (page - 1) * pageSize + index + 1 }}</td>
                <td class="muted">{{ new Date(item.created_at).toLocaleString() }}</td>
                <td>
                  <span class="tag-chip">{{ labelAction(item.action) }}</span>
                  <span class="table-cell-sub table-cell-sub--mono">{{ item.action }}</span>
                </td>
                <td>
                  <div class="table-cell-stack-body">
                    <span class="table-cell-main">{{ labelTargetType(item.target_type) }}</span>
                    <span class="table-cell-sub table-cell-sub--mono">{{ item.target_id || "-" }}</span>
                  </div>
                </td>
                <td>
                  <span class="tag-chip" :data-outcome="item.outcome">
                    {{ labelOutcome(item.outcome) }}
                  </span>
                </td>
                <td>
                  <div class="table-cell-stack-body">
                    <span class="table-cell-main">{{ operatorLabel(item) }}</span>
                    <span class="table-cell-sub table-cell-sub--mono">{{ item.actor_user_id || "-" }}</span>
                  </div>
                </td>
                <td>{{ item.ip_address || "-" }}</td>
                <td class="audit-user-agent" :title="item.user_agent || '-'">
                  {{ item.user_agent || "-" }}
                </td>
              </tr>
              <tr v-if="expandedId === item.id" class="audit-detail-row">
                <td colspan="8">
                  <div class="audit-detail-title">
                    <strong>详细信息</strong>
                    <span class="muted">请求 ID：{{ item.request_id || "-" }}</span>
                  </div>
                  <pre class="audit-detail-pre">{{ metadataText(item.metadata) }}</pre>
                </td>
              </tr>
            </template>
            <tr v-if="filteredLogs.length === 0" class="table-empty-row">
              <td colspan="8">
                <div class="empty-state">暂无审计日志数据</div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div v-if="!loading" class="table-statusbar">
        <span>{{ filteredLogs.length === 0 ? "暂无分页" : `第 ${page} / 共 ${totalPages} 页` }}</span>
        <span>本页 {{ pagedLogs.length }} 条</span>
        <span>筛选后 {{ filteredLogs.length }} 条 / 最近 {{ logs.length }} 条</span>
        <span class="table-statusbar-actions">
          <button class="btn btn-ghost btn-sm" type="button" :disabled="page <= 1" @click="prevPage">
            上一页
          </button>
          <button class="btn btn-ghost btn-sm" type="button" :disabled="page >= totalPages" @click="nextPage">
            下一页
          </button>
        </span>
      </div>
    </div>
  </section>
</template>

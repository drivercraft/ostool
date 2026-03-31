<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RouterLink } from "vue-router";

import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api/client";
import { useUiStore } from "@/stores/ui";
import type { BoardConfig, Session } from "@/types/api";

const ui = useUiStore();
const loading = ref(true);
const boards = ref<BoardConfig[]>([]);
const sessions = ref<Session[]>([]);
const typeFilter = ref("");
const tagFilter = ref("");
const statusFilter = ref<"all" | "available" | "leased" | "disabled">("all");

const leasedBoardIds = computed(() => new Set(sessions.value.map((session) => session.board_id)));
const boardTypes = computed(() =>
  Array.from(new Set(boards.value.map((board) => board.board_type))).sort(),
);

const filteredBoards = computed(() =>
  boards.value.filter((board) => {
    const leased = leasedBoardIds.value.has(board.id);
    if (typeFilter.value && board.board_type !== typeFilter.value) {
      return false;
    }
    if (tagFilter.value) {
      const query = tagFilter.value.toLowerCase();
      if (!board.tags.some((tag) => tag.toLowerCase().includes(query))) {
        return false;
      }
    }
    if (statusFilter.value === "available" && (leased || board.disabled)) {
      return false;
    }
    if (statusFilter.value === "leased" && !leased) {
      return false;
    }
    if (statusFilter.value === "disabled" && !board.disabled) {
      return false;
    }
    return true;
  }),
);

function boardTone(board: BoardConfig): "good" | "warn" | "danger" | "neutral" {
  if (board.disabled) {
    return "neutral";
  }
  if (leasedBoardIds.value.has(board.id)) {
    return "warn";
  }
  return "good";
}

function boardStatus(board: BoardConfig): string {
  if (board.disabled) {
    return "已禁用";
  }
  if (leasedBoardIds.value.has(board.id)) {
    return "已租出";
  }
  return "可用";
}

async function loadBoards() {
  loading.value = true;
  try {
    const [boardList, sessionList] = await Promise.all([api.listBoards(), api.listSessions()]);
    boards.value = boardList;
    sessions.value = sessionList.sessions;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  ui.clearMessages();
  void loadBoards();
});
</script>

<template>
  <section class="page-grid">
    <div class="panel">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">配置目录</p>
          <h3>单板单文件管理</h3>
        </div>
        <div class="toolbar-actions">
          <button class="ghost-button" @click="loadBoards">刷新</button>
          <RouterLink class="primary-button" to="/boards/new">新建开发板</RouterLink>
        </div>
      </div>

      <div class="toolbar-grid">
        <label class="field">
          <span>板型过滤</span>
          <select v-model="typeFilter">
            <option value="">全部</option>
            <option v-for="boardType in boardTypes" :key="boardType" :value="boardType">
              {{ boardType }}
            </option>
          </select>
        </label>
        <label class="field">
          <span>标签模糊筛选</span>
          <input v-model="tagFilter" placeholder="例如 lab / usb" />
        </label>
        <label class="field">
          <span>状态</span>
          <select v-model="statusFilter">
            <option value="all">全部</option>
            <option value="available">可用</option>
            <option value="leased">已租出</option>
            <option value="disabled">已禁用</option>
          </select>
        </label>
      </div>

      <div v-if="loading" class="empty-state">正在加载开发板列表...</div>
      <div v-else-if="filteredBoards.length === 0" class="empty-state">
        当前没有符合筛选条件的开发板。
      </div>
      <table v-else class="data-table">
        <thead>
          <tr>
            <th>ID</th>
            <th>板型</th>
            <th>标签</th>
            <th>串口</th>
            <th>启动方式</th>
            <th>状态</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="board in filteredBoards" :key="board.id">
            <td><code>{{ board.id }}</code></td>
            <td>{{ board.board_type }}</td>
            <td>{{ board.tags.join(", ") || "-" }}</td>
            <td>{{ board.serial ? `${board.serial.port} @ ${board.serial.baud_rate}` : "未配置" }}</td>
            <td>{{ board.boot.kind }}</td>
            <td>
              <StatusPill :tone="boardTone(board)" :label="boardStatus(board)" />
            </td>
            <td>
              <RouterLink class="inline-link" :to="`/boards/${board.id}`">编辑</RouterLink>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>

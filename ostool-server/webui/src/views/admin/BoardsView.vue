<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RouterLink } from "vue-router";

import { api } from "@/api";
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

const boardStats = computed(() => {
  const total = boards.value.length;
  const available = boards.value.filter((b) => !b.disabled && !leasedBoardIds.value.has(b.id)).length;
  const leased = boards.value.filter((b) => !b.disabled && leasedBoardIds.value.has(b.id)).length;
  const disabled = boards.value.filter((b) => b.disabled).length;
  return { total, available, leased, disabled };
});

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

function serialPrimaryLabel(board: BoardConfig): string {
  if (!board.serial) {
    return "";
  }
  return board.serial.key.kind === "serial_number" ? "SN" : "USB PATH";
}

function serialSecondaryLines(board: BoardConfig): string[] {
  if (!board.serial) {
    return [];
  }
  return [board.serial.resolved_usb_path, board.serial.resolved_device_path]
    .filter((value): value is string => Boolean(value))
    .filter((value, index, items) => items.indexOf(value) === index);
}

async function removeBoard(boardId: string) {
  if (!window.confirm(`确认删除开发板 ${boardId} 吗？`)) {
    return;
  }
  try {
    await api.deleteBoard(boardId);
    ui.setSuccess(`已删除开发板 ${boardId}`);
    await loadBoards();
  } catch (error) {
    ui.setError((error as Error).message);
  }
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
  <div class="space-y-6">
    <!-- Stats -->
    <div v-if="!loading" class="grid grid-cols-2 sm:grid-cols-4 gap-3">
      <div class="card p-4 text-center">
        <div class="text-2xl font-bold text-slate-700">{{ boardStats.total }}</div>
        <div class="text-xs text-slate-400 mt-0.5">全部</div>
      </div>
      <div class="card p-4 text-center border-emerald-200">
        <div class="text-2xl font-bold text-emerald-600">{{ boardStats.available }}</div>
        <div class="text-xs text-slate-400 mt-0.5">可用</div>
      </div>
      <div class="card p-4 text-center border-amber-200">
        <div class="text-2xl font-bold text-amber-600">{{ boardStats.leased }}</div>
        <div class="text-xs text-slate-400 mt-0.5">已租出</div>
      </div>
      <div class="card p-4 text-center">
        <div class="text-2xl font-bold text-slate-400">{{ boardStats.disabled }}</div>
        <div class="text-xs text-slate-400 mt-0.5">已禁用</div>
      </div>
    </div>

    <!-- Toolbar -->
    <div class="flex flex-col sm:flex-row gap-3 items-start sm:items-center justify-between">
      <div class="flex flex-wrap gap-2">
        <select v-model="typeFilter" class="select-field w-auto min-w-[140px] text-sm py-2">
          <option value="">全部型号</option>
          <option v-for="type in boardTypes" :key="type" :value="type">{{ type }}</option>
        </select>
        <input v-model="tagFilter" type="text" placeholder="标签筛选..." class="input-field w-auto min-w-[120px] text-sm py-2" />
        <select v-model="statusFilter" class="select-field w-auto min-w-[120px] text-sm py-2">
          <option value="all">全部状态</option>
          <option value="available">可用</option>
          <option value="leased">已租出</option>
          <option value="disabled">已禁用</option>
        </select>
      </div>
      <div class="flex gap-2">
        <button class="btn-secondary btn-sm" @click="loadBoards">刷新</button>
        <RouterLink to="/admin/resources/boards/new" class="btn-primary btn-sm">新增开发板</RouterLink>
      </div>
    </div>

    <!-- Loading / Empty -->
    <div v-if="loading" class="card p-12 flex flex-col items-center justify-center">
      <div class="w-8 h-8 border-2 border-indigo-500 border-t-transparent rounded-full animate-spin mb-3" />
      <p class="text-slate-400 text-sm">正在加载开发板列表...</p>
    </div>

    <div v-else-if="filteredBoards.length === 0" class="card p-12 flex flex-col items-center justify-center text-center">
      <div class="text-3xl mb-3 text-slate-300">&#9641;</div>
      <p class="text-slate-500 text-sm">当前没有符合筛选条件的开发板</p>
    </div>

    <!-- Table -->
    <div v-else class="table-container">
      <table class="data-table">
        <thead>
          <tr>
            <th>开发板 ID</th>
            <th>型号</th>
            <th>标签</th>
            <th>串口</th>
            <th>状态</th>
            <th class="text-right">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="board in filteredBoards" :key="board.id">
            <td class="font-mono text-xs">{{ board.id }}</td>
            <td class="font-medium">{{ board.board_type }}</td>
            <td>
              <div class="flex flex-wrap gap-1">
                <span v-for="tag in board.tags" :key="tag" class="tag">{{ tag }}</span>
                <span v-if="board.tags.length === 0" class="text-xs text-slate-400">-</span>
              </div>
            </td>
            <td class="text-xs text-slate-500">{{ serialPrimaryLabel(board) || '-' }}</td>
            <td>
              <span :class="boardTone(board) === 'good' ? 'pill-success' : boardTone(board) === 'warn' ? 'pill-warning' : 'pill-neutral'">
                {{ boardStatus(board) }}
              </span>
            </td>
            <td class="text-right">
              <div class="flex items-center justify-end gap-1">
                <RouterLink :to="`/admin/resources/boards/${board.id}/edit`" class="btn-ghost btn-sm text-xs">编辑</RouterLink>
                <button class="btn-ghost btn-sm text-xs text-red-500 hover:text-red-600 hover:bg-red-50" @click="removeBoard(board.id)">删除</button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>

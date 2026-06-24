<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RouterLink } from "vue-router";

import Icon from "@/components/Icon.vue";
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
  const confirmed = await ui.confirm({
    tone: "danger",
    title: "删除开发板",
    message: `确认删除开发板 ${boardId} 吗？`,
    confirmLabel: "删除",
  });
  if (!confirmed) {
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
  <div class="boards-page page-grid">
    <section class="admin-toolbar">
      <div class="admin-toolbar-left">
        <label class="field filter-field">
          <span>开发板型号</span>
          <select v-model="typeFilter" aria-label="开发板型号">
            <option value="">全部型号</option>
            <option v-for="type in boardTypes" :key="type" :value="type">{{ type }}</option>
          </select>
        </label>
        <label class="search-field">
          <Icon name="search" :size="16" />
          <input
            v-model="tagFilter"
            type="search"
            placeholder="搜索标签..."
            aria-label="标签筛选"
          />
        </label>
        <label class="field filter-field">
          <span>开发板状态</span>
          <select v-model="statusFilter" aria-label="开发板状态">
            <option value="all">全部状态</option>
            <option value="available">可用</option>
            <option value="leased">已租出</option>
            <option value="disabled">已禁用</option>
          </select>
        </label>
      </div>
      <div class="admin-toolbar-right">
        <button class="btn btn-secondary btn-sm" @click="loadBoards">刷新</button>
        <RouterLink to="/admin/resources/boards/new" class="btn btn-primary btn-sm">新增开发板</RouterLink>
      </div>
    </section>

    <!-- Loading / Empty -->
    <div class="panel admin-table-panel">
      <div v-if="loading" class="empty-state">
        <div class="spinner"></div>
        正在加载开发板列表...
      </div>

      <div v-else-if="filteredBoards.length === 0" class="empty-state">
        <div class="empty-state-icon">&#9641;</div>
        当前没有符合筛选条件的开发板
      </div>

      <!-- Table -->
      <div v-else class="table-scroll">
        <table class="data-table">
        <thead>
          <tr>
            <th>开发板 ID</th>
            <th>型号</th>
            <th>标签</th>
            <th>串口</th>
            <th>状态</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="board in filteredBoards" :key="board.id">
            <td><code>{{ board.id }}</code></td>
            <td><strong>{{ board.board_type }}</strong></td>
            <td>
              <div class="tag-list">
                <span v-for="tag in board.tags" :key="tag" class="tag">{{ tag }}</span>
                <span v-if="board.tags.length === 0" class="muted">-</span>
              </div>
            </td>
            <td class="muted">{{ serialPrimaryLabel(board) || '-' }}</td>
            <td>
              <span
                class="pill"
                :class="boardTone(board) === 'good' ? 'pill-success' : boardTone(board) === 'warn' ? 'pill-warning' : 'pill-neutral'"
              >
                {{ boardStatus(board) }}
              </span>
            </td>
            <td>
              <div class="row-actions">
                <RouterLink
                  :to="{ name: 'admin-resource-board-edit', params: { boardId: board.id } }"
                  class="btn btn-ghost btn-sm"
                >
                  编辑
                </RouterLink>
                <button class="btn btn-danger btn-sm" @click="removeBoard(board.id)">删除</button>
              </div>
            </td>
          </tr>
        </tbody>
        </table>
      </div>
    </div>
  </div>
</template>

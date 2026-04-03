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
  <section class="page-grid">
    <!-- Stats strip -->
    <div class="stats-strip" v-if="!loading">
      <div class="stats-chip">
        <span class="stats-num">{{ boardStats.total }}</span>
        <span class="stats-label">全部</span>
      </div>
      <div class="stats-chip stats-chip-good">
        <span class="stats-num">{{ boardStats.available }}</span>
        <span class="stats-label">可用</span>
      </div>
      <div class="stats-chip stats-chip-warn">
        <span class="stats-num">{{ boardStats.leased }}</span>
        <span class="stats-label">已租出</span>
      </div>
      <div class="stats-chip stats-chip-neutral">
        <span class="stats-num">{{ boardStats.disabled }}</span>
        <span class="stats-label">已禁用</span>
      </div>
    </div>

    <div class="panel">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">配置目录</p>
          <h3>开发板管理</h3>
        </div>
        <div class="toolbar-actions">
          <button class="ghost-button" @click="loadBoards">刷新</button>
          <RouterLink class="primary-button" to="/boards/new">新建开发板</RouterLink>
        </div>
      </div>

      <!-- Filter bar -->
      <div class="filter-bar">
        <label class="field filter-field">
          <span>板型</span>
          <select v-model="typeFilter">
            <option value="">全部</option>
            <option v-for="boardType in boardTypes" :key="boardType" :value="boardType">
              {{ boardType }}
            </option>
          </select>
        </label>
        <label class="field filter-field">
          <span>标签</span>
          <input v-model="tagFilter" placeholder="模糊搜索..." />
        </label>
        <label class="field filter-field">
          <span>状态</span>
          <select v-model="statusFilter">
            <option value="all">全部</option>
            <option value="available">可用</option>
            <option value="leased">已租出</option>
            <option value="disabled">已禁用</option>
          </select>
        </label>
      </div>

      <div v-if="loading" class="empty-state">
        <div class="empty-state-icon">&#9641;</div>
        正在加载开发板列表...
      </div>
      <div v-else-if="filteredBoards.length === 0" class="empty-state">
        <div class="empty-state-icon">&#9641;</div>
        当前没有符合筛选条件的开发板。
      </div>

      <!-- Board card grid -->
      <div v-else class="board-card-grid">
        <div v-for="board in filteredBoards" :key="board.id" class="board-card">
          <div class="board-card-header">
            <div class="board-card-id">
              <span class="board-card-status-dot" :data-tone="boardTone(board)" />
              <div>
                <code>{{ board.id }}</code>
                <div class="board-card-type">{{ board.board_type }}</div>
              </div>
            </div>
            <StatusPill :tone="boardTone(board)" :label="boardStatus(board)" />
          </div>

          <div v-if="board.tags.length" class="board-card-tags">
            <span v-for="tag in board.tags" :key="tag" class="tag-chip">{{ tag }}</span>
          </div>

          <div class="board-card-meta">
            <div class="board-card-meta-item">
              <span class="board-card-meta-label">串口</span>
              <div v-if="board.serial" class="board-card-serial-mini">
                <span class="serial-key-badge" style="width: fit-content">{{ serialPrimaryLabel(board) }}</span>
                <strong style="font-size: 0.88rem; line-break: anywhere">{{ board.serial.key.value }}</strong>
                <span style="color: var(--muted); font-size: 0.8rem">@ {{ board.serial.baud_rate }}</span>
              </div>
              <span v-else style="color: var(--muted); font-size: 0.85rem">未配置</span>
            </div>
            <div class="board-card-meta-item">
              <span class="board-card-meta-label">启动方式</span>
              <span>{{ board.boot.kind }}</span>
            </div>
          </div>

          <div class="board-card-actions">
            <RouterLink class="ghost-button compact-button" :to="`/boards/${board.id}`">编辑配置</RouterLink>
            <button class="danger-button compact-button" @click="removeBoard(board.id)">删除</button>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.stats-strip {
  display: flex;
  gap: 14px;
}

.stats-chip {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 20px;
  border-radius: 16px;
  background: var(--panel);
  border: 1px solid rgba(96, 79, 53, 0.12);
}

.stats-chip-good {
  border-color: rgba(22, 97, 75, 0.18);
  background: var(--good-soft);
}

.stats-chip-warn {
  border-color: rgba(185, 115, 22, 0.18);
  background: var(--warn-soft);
}

.stats-chip-neutral {
  border-color: rgba(139, 127, 115, 0.18);
  background: var(--neutral-soft);
}

.stats-num {
  font-size: 1.2rem;
  font-weight: 700;
}

.stats-label {
  font-size: 0.84rem;
  color: var(--muted);
}

.filter-bar {
  display: flex;
  gap: 14px;
  margin-bottom: 22px;
  padding: 16px 20px;
  border-radius: 16px;
  background: rgba(255, 255, 255, 0.4);
  border: 1px solid rgba(96, 79, 53, 0.08);
}

.filter-field {
  flex: 1;
  min-width: 0;
}

.filter-field span {
  font-size: 0.8rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--muted);
}

.filter-field select,
.filter-field input {
  padding: 8px 12px;
  border-radius: 10px;
  font-size: 0.88rem;
}

@media (max-width: 860px) {
  .stats-strip {
    flex-wrap: wrap;
  }

  .filter-bar {
    flex-direction: column;
  }
}
</style>

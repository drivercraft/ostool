<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { RouterLink, useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { AdminSessionResponse, BoardConfig, LeaseResponse } from "@/types/api";

const ui = useUiStore();
const router = useRouter();
const loading = ref(true);
const boards = ref<BoardConfig[]>([]);
const sessions = ref<AdminSessionResponse[]>([]);
const leases = ref<LeaseResponse[]>([]);
const typeFilter = ref("");
const tagFilter = ref("");
const statusFilter = ref<"all" | "idle" | "leased" | "in_use" | "disabled">("all");
const openMenuBoardId = ref<string | null>(null);
const menuPosition = ref({ top: 0, left: 0 });

const activeSessionBoardIds = computed(() =>
  new Set(
    sessions.value
      .filter((item) => {
        const expiresAt = new Date(item.session.expires_at).getTime();
        return (item.session.state === "active" || item.session.state === "releasing")
          && Number.isFinite(expiresAt)
          && Date.now() < expiresAt;
      })
      .map((item) => item.session.board_id),
  ),
);
const currentLeaseBoardIds = computed(() => {
  const now = Date.now();
  return new Set(
    leases.value
      .filter((item) => {
        const start = new Date(item.lease.starts_at).getTime();
        const end = new Date(item.lease.expires_at).getTime();
        return (item.lease.state === "active" || item.lease.state === "releasing")
          && Number.isFinite(start)
          && Number.isFinite(end)
          && start <= now
          && now < end;
      })
      .map((item) => item.lease.board_id),
  );
});
const leaseByBoardId = computed(() => {
  const map = new Map<string, LeaseResponse["lease"]>();
  const score = (lease: LeaseResponse["lease"]) => {
    if (lease.state === "active") {
      return 3;
    }
    if (lease.state === "releasing") {
      return 2;
    }
    return 1;
  };
  for (const item of leases.value) {
    const current = map.get(item.lease.board_id);
    if (
      !current
      || score(item.lease) > score(current)
      || (score(item.lease) === score(current)
        && new Date(item.lease.starts_at).getTime() > new Date(current.starts_at).getTime())
    ) {
      map.set(item.lease.board_id, item.lease);
    }
  }
  return map;
});
const boardTypes = computed(() =>
  Array.from(new Set(boards.value.map((board) => board.board_type))).sort(),
);

const filteredBoards = computed(() =>
  boards.value.filter((board) => {
    const status = boardStatusState(board);
    if (typeFilter.value && board.board_type !== typeFilter.value) {
      return false;
    }
    if (tagFilter.value) {
      const query = tagFilter.value.toLowerCase();
      if (!board.tags.some((tag) => tag.toLowerCase().includes(query))) {
        return false;
      }
    }
    if (statusFilter.value !== "all" && status !== statusFilter.value) {
      return false;
    }
    return true;
  }),
);

function boardStatusState(board: BoardConfig): "idle" | "leased" | "in_use" | "disabled" {
  if (board.disabled) {
    return "disabled";
  }
  if (activeSessionBoardIds.value.has(board.id)) {
    return "in_use";
  }
  if (currentLeaseBoardIds.value.has(board.id)) {
    return "leased";
  }
  return "idle";
}

function boardTone(board: BoardConfig): "good" | "warn" | "danger" | "neutral" {
  const status = boardStatusState(board);
  if (status === "idle") {
    return "good";
  }
  if (status === "leased") {
    return "warn";
  }
  if (status === "in_use") {
    return "danger";
  }
  return "neutral";
}

function boardStatus(board: BoardConfig): string {
  const labels = {
    idle: "空闲中",
    leased: "已租赁",
    in_use: "使用中",
    disabled: "已禁用",
  };
  return labels[boardStatusState(board)];
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

function boardToUpsertRequest(board: BoardConfig, disabled = board.disabled) {
  return {
    id: board.id,
    board_type: board.board_type,
    tags: board.tags,
    notes: board.notes,
    disabled,
    serial: board.serial,
    power_management: board.power_management,
    boot: board.boot,
  };
}

function toggleMenu(boardId: string, event: MouseEvent) {
  if (openMenuBoardId.value === boardId) {
    closeMenu();
    return;
  }
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  menuPosition.value = {
    top: rect.bottom + 6,
    left: Math.max(12, rect.right - 180),
  };
  openMenuBoardId.value = boardId;
}

function closeMenu() {
  openMenuBoardId.value = null;
}

function onDocumentClick(event: MouseEvent) {
  const target = event.target as HTMLElement | null;
  if (target && !target.closest(".row-action-menu") && !target.closest(".action-menu")) {
    closeMenu();
  }
}

async function toggleDisabled(board: BoardConfig) {
  if (activeSessionBoardIds.value.has(board.id)) {
    return;
  }
  closeMenu();
  try {
    const updated = await api.admin.updateBoard(board.id, boardToUpsertRequest(board, !board.disabled));
    boards.value = boards.value.map((item) => (item.id === board.id ? updated : item));
    ui.setSuccess(updated.disabled ? `已禁用开发板 ${updated.id}` : `已启用开发板 ${updated.id}`);
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

function goToBoardLease(boardId: string) {
  const lease = leaseByBoardId.value.get(boardId);
  if (!lease) {
    return;
  }
  closeMenu();
  void router.push({
    path: "/admin/rentals/leases",
    query: { q: lease.id },
  });
}

async function removeBoard(boardId: string) {
  closeMenu();
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
    await api.admin.deleteBoard(boardId);
    ui.setSuccess(`已删除开发板 ${boardId}`);
    await loadBoards();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

async function loadBoards() {
  loading.value = true;
  try {
    const [boardList, sessionList, leaseList] = await Promise.all([
      api.admin.listBoards(),
      api.admin.listSessions(),
      api.admin.listAdminLeases(),
    ]);
    boards.value = boardList;
    sessions.value = sessionList.sessions;
    leases.value = leaseList.leases;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  ui.clearMessages();
  document.addEventListener("click", onDocumentClick);
  void loadBoards();
});

onUnmounted(() => document.removeEventListener("click", onDocumentClick));
</script>

<template>
  <div class="boards-page page-grid admin-list-page">
    <div class="admin-table-panel">
      <section class="admin-toolbar">
        <div class="admin-toolbar-left">
          <RouterLink to="/admin/resources/boards/new" class="btn btn-primary btn-sm">新增开发板</RouterLink>
        </div>
        <div class="admin-toolbar-right">
          <label class="search-field">
            <Icon name="search" :size="16" />
            <input
              v-model="tagFilter"
              type="search"
              maxlength="128"
              placeholder="搜索标签..."
              aria-label="标签筛选"
            />
          </label>
          <label class="field select-field filter-field">
            <span>开发板型号</span>
            <select v-model="typeFilter" aria-label="开发板型号">
              <option value="">全部型号</option>
              <option v-for="type in boardTypes" :key="type" :value="type">{{ type }}</option>
            </select>
          </label>
          <label class="field select-field filter-field">
            <span>开发板状态</span>
            <select v-model="statusFilter" aria-label="开发板状态">
              <option value="all">全部状态</option>
              <option value="idle">空闲中</option>
              <option value="leased">已租赁</option>
              <option value="in_use">使用中</option>
              <option value="disabled">已禁用</option>
            </select>
          </label>
        </div>
      </section>

      <div v-if="loading" class="empty-state">
        <div class="spinner"></div>
        正在加载开发板列表...
      </div>

      <!-- Table -->
      <div v-else class="table-scroll">
        <table class="data-table">
        <thead>
          <tr>
            <th class="col-index">序号</th>
            <th>开发板 ID</th>
            <th>型号</th>
            <th>标签</th>
            <th>串口</th>
            <th>状态</th>
            <th class="col-actions">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(board, index) in filteredBoards" :key="board.id">
            <td class="col-index">{{ index + 1 }}</td>
            <td>{{ board.id }}</td>
            <td>{{ board.board_type }}</td>
            <td>
              <div class="tag-list">
                <span v-for="tag in board.tags" :key="tag" class="tag">{{ tag }}</span>
                <span v-if="board.tags.length === 0" class="muted">-</span>
              </div>
            </td>
            <td class="muted">{{ serialPrimaryLabel(board) || '-' }}</td>
            <td class="col-actions">
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
                  class="btn-icon-only"
                  title="编辑"
                >
                  <Icon name="edit" :size="16" />
                </RouterLink>
                <button
                  class="btn-icon-only"
                  :title="board.disabled ? '启用' : '禁用'"
                  :disabled="activeSessionBoardIds.has(board.id)"
                  @click="toggleDisabled(board)"
                >
                  <Icon :name="board.disabled ? 'check' : 'ban'" :size="16" />
                </button>
                <div class="row-action-menu">
                  <button
                    class="btn-icon-only"
                    title="更多"
                    :aria-expanded="openMenuBoardId === board.id"
                    @click.stop="toggleMenu(board.id, $event)"
                  >
                    <Icon name="more-vertical" :size="16" />
                  </button>
                </div>
                <Teleport to="body">
                  <div
                    v-if="openMenuBoardId === board.id"
                    class="action-menu action-menu--floating"
                    :style="{ top: `${menuPosition.top}px`, left: `${menuPosition.left}px` }"
                  >
                    <button
                      class="action-menu-item"
                      :disabled="!leaseByBoardId.has(board.id)"
                      @click="goToBoardLease(board.id)"
                    >
                      <Icon name="link" :size="14" />
                      转到租赁
                    </button>
                    <button
                      class="action-menu-item"
                      @click="removeBoard(board.id)"
                    >
                      <Icon name="trash" :size="14" />
                      删除开发板
                    </button>
                  </div>
                </Teleport>
              </div>
            </td>
          </tr>
          <tr v-if="filteredBoards.length === 0" class="table-empty-row">
            <td colspan="7">
              <div class="empty-state">暂无开发板数据</div>
            </td>
          </tr>
        </tbody>
        </table>
      </div>

      <div v-if="!loading" class="table-statusbar">
        <span>{{ filteredBoards.length === 0 ? "暂无分页" : "第 1 / 共 1 页" }}</span>
        <span>本页 {{ filteredBoards.length }} 条</span>
        <span>筛选后 {{ filteredBoards.length }} 条 / 共 {{ boards.length }} 条</span>
      </div>
    </div>
  </div>
</template>

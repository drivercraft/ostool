<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { AdminUserResponse, BoardConfig, LeaseResponse } from "@/types/api";

type LeaseModalMode = "create" | "edit" | null;
type LeaseStateFilter = "all" | "active" | "releasing" | "released" | "expired" | "failed";

const ui = useUiStore();
const router = useRouter();
const leases = ref<LeaseResponse[]>([]);
const users = ref<AdminUserResponse[]>([]);
const boards = ref<BoardConfig[]>([]);
const loading = ref(true);
const saving = ref(false);
const modalMode = ref<LeaseModalMode>(null);
const editingLease = ref<LeaseResponse | null>(null);
const pointerDownOnModalOverlay = ref(false);
const openMenuLeaseId = ref<string | null>(null);
const menuPosition = ref({ top: 0, left: 0 });
const search = ref("");
const stateFilter = ref<LeaseStateFilter>("all");
const form = ref({
  user_id: "",
  board_id: "",
  client_name: "",
  starts_at: "",
  expires_at: "",
  failure_message: "",
});

const availableBoards = computed(() => {
  const activeBoardIds = new Set(
    leases.value
      .filter((item) => item.lease.state === "active" && windowsOverlap(item.lease.starts_at, item.lease.expires_at, form.value.starts_at, form.value.expires_at))
      .map((item) => item.lease.board_id),
  );
  return boards.value.filter((board) => !board.disabled && !activeBoardIds.has(board.id));
});

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
      userLabel(item.lease.user_id),
      item.lease.board_id,
      item.lease.board_type,
      item.lease.session_id ?? "",
      item.session?.client_name ?? "",
    ].some((value) => value.toLowerCase().includes(query));
  }),
);

function userLabel(userId: string) {
  const user = users.value.find((item) => item.id === userId);
  if (!user) {
    return userId;
  }
  return user.display_name || user.username;
}

function formatDateTime(value: string) {
  return new Date(value).toLocaleString();
}

function formatDuration(start: string, end: string) {
  const ms = new Date(end).getTime() - new Date(start).getTime();
  if (!Number.isFinite(ms) || ms <= 0) {
    return "-";
  }
  const minutes = Math.round(ms / 60000);
  if (minutes < 60) {
    return `${minutes} 分钟`;
  }
  const hours = Math.floor(minutes / 60);
  const remain = minutes % 60;
  return remain ? `${hours} 小时 ${remain} 分钟` : `${hours} 小时`;
}

function toDatetimeLocal(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "";
  }
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60000);
  return local.toISOString().slice(0, 16);
}

function fromDatetimeLocal(value: string) {
  return new Date(value).toISOString();
}

function defaultStartsAt() {
  return toDatetimeLocal(new Date().toISOString());
}

function defaultExpiresAt(startsAt: string) {
  const startTime = startsAt ? new Date(startsAt).getTime() : Date.now();
  return toDatetimeLocal(new Date(startTime + 60 * 60 * 1000).toISOString());
}

function windowsOverlap(startA: string, endA: string, startB: string, endB: string) {
  if (!startB || !endB) {
    return true;
  }
  return new Date(startA).getTime() < new Date(endB).getTime()
    && new Date(endA).getTime() > new Date(startB).getTime();
}

function leaseTone(state: string) {
  if (state === "active") {
    return "good";
  }
  if (state === "failed") {
    return "danger";
  }
  return "neutral";
}

function isActiveLease(item: LeaseResponse) {
  return item.lease.state === "active";
}

function canStartLeaseSession(item: LeaseResponse) {
  const now = Date.now();
  return isActiveLease(item)
    && !item.session
    && new Date(item.lease.starts_at).getTime() <= now
    && now < new Date(item.lease.expires_at).getTime();
}

function canToggleLease(item: LeaseResponse) {
  return Boolean(item.session) || canStartLeaseSession(item);
}

function toggleLeaseTitle(item: LeaseResponse) {
  if (item.session) {
    return "禁用";
  }
  return "启用";
}

function toggleMenu(leaseId: string, event: MouseEvent) {
  if (openMenuLeaseId.value === leaseId) {
    closeMenu();
    return;
  }
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  menuPosition.value = {
    top: rect.bottom + 6,
    left: Math.max(12, rect.right - 180),
  };
  openMenuLeaseId.value = leaseId;
}

function closeMenu() {
  openMenuLeaseId.value = null;
}

function onDocumentClick(event: MouseEvent) {
  const target = event.target as HTMLElement | null;
  if (target && !target.closest(".row-action-menu") && !target.closest(".action-menu")) {
    closeMenu();
  }
}

function goToSession(item: LeaseResponse) {
  if (!item.session) {
    return;
  }
  closeMenu();
  void router.push({
    path: "/admin/rentals/sessions",
    query: { q: item.session.id },
  });
}

async function loadData() {
  loading.value = true;
  try {
    const [leaseResponse, userResponse, boardResponse] = await Promise.all([
      api.listAdminLeases(),
      api.listAdminUsers(),
      api.listBoards(),
    ]);
    leases.value = leaseResponse.leases;
    users.value = userResponse.users;
    boards.value = boardResponse;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

function openCreate() {
  const startsAt = defaultStartsAt();
  editingLease.value = null;
  form.value = {
    user_id: users.value.find((user) => !user.disabled)?.id ?? "",
    board_id: "",
    client_name: "",
    starts_at: startsAt,
    expires_at: defaultExpiresAt(startsAt),
    failure_message: "",
  };
  form.value.board_id = availableBoards.value[0]?.id ?? "";
  modalMode.value = "create";
}

function openEdit(item: LeaseResponse) {
  closeMenu();
  editingLease.value = item;
  form.value = {
    user_id: item.lease.user_id,
    board_id: item.lease.board_id,
    client_name: item.session?.client_name ?? "",
    starts_at: toDatetimeLocal(item.lease.starts_at),
    expires_at: toDatetimeLocal(item.lease.expires_at),
    failure_message: item.lease.failure_message ?? "",
  };
  modalMode.value = "edit";
}

function closeModal() {
  modalMode.value = null;
  editingLease.value = null;
}

function onModalOverlayPointerDown(event: PointerEvent) {
  pointerDownOnModalOverlay.value = event.target === event.currentTarget;
}

function onModalOverlayClick(event: MouseEvent) {
  if (pointerDownOnModalOverlay.value && event.target === event.currentTarget) {
    closeModal();
  }
  pointerDownOnModalOverlay.value = false;
}

async function submitLease() {
  if (!form.value.starts_at || !form.value.expires_at) {
    ui.setError("请填写租赁开始和结束时间");
    return;
  }
  if (new Date(form.value.expires_at).getTime() <= new Date(form.value.starts_at).getTime()) {
    ui.setError("租赁结束时间必须晚于开始时间");
    return;
  }
  saving.value = true;
  try {
    if (modalMode.value === "create") {
      if (!form.value.user_id || !form.value.board_id) {
        ui.setError("请选择用户和开发板");
        return;
      }
      await api.createAdminLease({
        user_id: form.value.user_id,
        board_id: form.value.board_id,
        client_name: form.value.client_name.trim() || null,
        starts_at: fromDatetimeLocal(form.value.starts_at),
        expires_at: fromDatetimeLocal(form.value.expires_at),
      });
      ui.setSuccess("租赁已创建");
    } else if (modalMode.value === "edit" && editingLease.value) {
      await api.updateAdminLease(editingLease.value.lease.id, {
        starts_at: fromDatetimeLocal(form.value.starts_at),
        expires_at: fromDatetimeLocal(form.value.expires_at),
        failure_message: form.value.failure_message.trim() || null,
      });
      ui.setSuccess("租赁已更新");
    }
    closeModal();
    await loadData();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    saving.value = false;
  }
}

async function startLeaseSession(item: LeaseResponse) {
  closeMenu();
  try {
    const updated = await api.startAdminLeaseSession(item.lease.id);
    leases.value = leases.value.map((lease) => (lease.lease.id === item.lease.id ? updated : lease));
    ui.setSuccess(`已启用租赁 ${item.lease.id}`);
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

async function confirmLeaseRemoval(leaseId: string, action: "disable" | "delete") {
  closeMenu();
  const actionLabel = action === "disable" ? "禁用" : "删除";
  const confirmed = await ui.confirm({
    tone: "danger",
    title: `${actionLabel}租赁`,
    message: `确认${actionLabel}租赁 ${leaseId}？`,
    confirmLabel: actionLabel,
  });
  if (!confirmed) {
    return;
  }
  try {
    await api.deleteAdminLease(leaseId);
    ui.setSuccess(`已发起${actionLabel}租赁 ${leaseId}`);
    await loadData();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

async function disableLease(leaseId: string) {
  await confirmLeaseRemoval(leaseId, "disable");
}

async function deleteLease(leaseId: string) {
  await confirmLeaseRemoval(leaseId, "delete");
}

onMounted(() => {
  document.addEventListener("click", onDocumentClick);
  void loadData();
});

onUnmounted(() => document.removeEventListener("click", onDocumentClick));
</script>

<template>
  <section class="page-grid admin-list-page">
    <div class="panel admin-table-panel">
      <div class="admin-toolbar">
        <div class="admin-toolbar-left">
          <button class="btn btn-primary" @click="openCreate">
            <Icon name="plus" :size="16" class="btn-icon" />
            新增租赁
          </button>
          <button class="btn btn-ghost btn-sm" @click="loadData">刷新</button>
        </div>
        <div class="admin-toolbar-right">
          <label class="search-field">
            <Icon name="search" :size="16" />
            <input v-model="search" type="search" placeholder="搜索租赁 / 用户 / 开发板" />
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
              <th>用户</th>
              <th>开发板</th>
              <th>会话</th>
              <th>状态</th>
              <th>租赁时间段</th>
              <th>时长</th>
              <th class="col-actions">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(item, index) in filteredLeases" :key="item.lease.id">
              <td class="col-index">{{ index + 1 }}</td>
              <td><code>{{ item.lease.id }}</code></td>
              <td>
                <strong>{{ userLabel(item.lease.user_id) }}</strong>
                <div><code>{{ item.lease.user_id }}</code></div>
              </td>
              <td>
                <code>{{ item.lease.board_id }}</code>
                <div class="muted">{{ item.lease.board_type }}</div>
              </td>
              <td><code>{{ item.lease.session_id || "-" }}</code></td>
              <td>
                <StatusPill :tone="leaseTone(item.lease.state)" :label="item.lease.state" />
              </td>
              <td>
                <div>{{ formatDateTime(item.lease.starts_at) }}</div>
                <div class="muted">至 {{ formatDateTime(item.lease.expires_at) }}</div>
              </td>
              <td>{{ formatDuration(item.lease.starts_at, item.lease.expires_at) }}</td>
              <td class="col-actions">
                <div class="row-actions">
                  <button
                    class="btn-icon-only"
                    title="编辑"
                    :disabled="!isActiveLease(item)"
                    @click="openEdit(item)"
                  >
                    <Icon name="edit" :size="16" />
                  </button>
                  <button
                    class="btn-icon-only"
                    :title="toggleLeaseTitle(item)"
                    :disabled="!canToggleLease(item)"
                    @click="item.session ? disableLease(item.lease.id) : startLeaseSession(item)"
                  >
                    <Icon :name="item.session ? 'ban' : 'check'" :size="16" />
                  </button>
                  <div class="row-action-menu">
                    <button
                      class="btn-icon-only"
                      title="更多"
                      :aria-expanded="openMenuLeaseId === item.lease.id"
                      @click.stop="toggleMenu(item.lease.id, $event)"
                    >
                      <Icon name="more-vertical" :size="16" />
                    </button>
                  </div>
                  <Teleport to="body">
                    <div
                      v-if="openMenuLeaseId === item.lease.id"
                      class="action-menu action-menu--floating"
                      :style="{ top: `${menuPosition.top}px`, left: `${menuPosition.left}px` }"
                    >
                      <button
                        class="action-menu-item"
                        :disabled="!item.session"
                        @click="goToSession(item)"
                      >
                        <Icon name="terminal" :size="14" />
                        转到会话
                      </button>
                      <button
                        class="action-menu-item"
                        :disabled="!isActiveLease(item)"
                        @click="deleteLease(item.lease.id)"
                      >
                        <Icon name="trash" :size="14" />
                        删除租赁
                      </button>
                    </div>
                  </Teleport>
                </div>
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

  <div
    v-if="modalMode"
    class="modal-overlay"
    @pointerdown="onModalOverlayPointerDown"
    @click="onModalOverlayClick"
  >
    <div class="modal-card modal-card--user-form">
      <header class="modal-header">
        <div>
          <h3>{{ modalMode === "create" ? "新增租赁" : `编辑租赁 ${editingLease?.lease.id}` }}</h3>
        </div>
        <button class="btn-icon-only modal-close-button" title="关闭" @click="closeModal">×</button>
      </header>

      <form class="modal-form" @submit.prevent="submitLease">
        <div class="modal-body modal-body-grid">
          <template v-if="modalMode === 'create'">
            <label class="field">
              <span>用户</span>
              <select v-model="form.user_id">
                <option value="" disabled>请选择用户</option>
                <option
                  v-for="user in users.filter((item) => !item.disabled)"
                  :key="user.id"
                  :value="user.id"
                >
                  {{ user.display_name || user.username }} / {{ user.username }}
                </option>
              </select>
            </label>
            <label class="field">
              <span>开发板</span>
              <select v-model="form.board_id">
                <option value="" disabled>请选择空闲开发板</option>
                <option v-for="board in availableBoards" :key="board.id" :value="board.id">
                  {{ board.id }} / {{ board.board_type }}
                </option>
              </select>
            </label>
            <label class="field modal-field-full">
              <span>会话名称（选填）</span>
              <input v-model="form.client_name" placeholder="例如 手动分配给 Alice" />
            </label>
          </template>
          <template v-else>
            <label class="field">
              <span>用户</span>
              <input :value="userLabel(form.user_id)" disabled />
            </label>
            <label class="field">
              <span>开发板</span>
              <input :value="form.board_id" disabled />
            </label>
            <label class="field modal-field-full">
              <span>备注 / 失败信息</span>
              <input v-model="form.failure_message" placeholder="可选" />
            </label>
          </template>
          <label class="field">
            <span>租赁开始时间</span>
            <input v-model="form.starts_at" type="datetime-local" />
          </label>
          <label class="field">
            <span>租赁结束时间</span>
            <input v-model="form.expires_at" type="datetime-local" />
          </label>
        </div>

        <div class="modal-actions">
          <button type="submit" class="btn btn-primary" :disabled="saving">
            {{ saving ? "保存中..." : "保存" }}
          </button>
          <button type="button" class="btn btn-ghost" :disabled="saving" @click="closeModal">取消</button>
        </div>
      </form>
    </div>
  </div>
</template>

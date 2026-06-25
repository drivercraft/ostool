<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import type { AdminUserResponse, BoardConfig, LeaseResponse } from "@/types/api";

type LeaseStateFilter = "all" | "active" | "releasing" | "released" | "expired" | "failed";

const ui = useUiStore();
const auth = useAuthStore();
const router = useRouter();
const leases = ref<LeaseResponse[]>([]);
const users = ref<AdminUserResponse[]>([]);
const boards = ref<BoardConfig[]>([]);
const loading = ref(true);
const openMenuLeaseId = ref<string | null>(null);
const menuPosition = ref({ top: 0, left: 0 });
const search = ref("");
const stateFilter = ref<LeaseStateFilter>("all");
const canDeleteLeases = computed(() => auth.hasPermission("leases.delete"));
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

function leaseTone(state: string) {
  if (state === "active") {
    return "good";
  }
  if (state === "failed") {
    return "danger";
  }
  return "neutral";
}

function leaseStateLabel(state: string) {
  const labels: Record<string, string> = {
    active: "生效中",
    releasing: "释放中",
    released: "已释放",
    expired: "已过期",
    failed: "失败",
  };
  return labels[state] ?? state;
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
  const sessionId = item.session?.id || item.lease.session_id;
  if (!sessionId) {
    return;
  }
  closeMenu();
  void router.push({
    path: "/admin/rentals/sessions",
    query: { q: sessionId },
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
  void router.push({ name: "admin-rental-lease-new" });
}

function openEdit(item: LeaseResponse) {
  closeMenu();
  void router.push({ name: "admin-rental-lease-edit", params: { leaseId: item.lease.id } });
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
  if (action === "delete" && !canDeleteLeases.value) {
    ui.setError("缺少删除租赁数据权限");
    return;
  }
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
    if (action === "disable") {
      await api.releaseAdminLease(leaseId);
      ui.setSuccess(`已发起${actionLabel}租赁 ${leaseId}`);
    } else {
      await api.deleteAdminLease(leaseId);
      ui.setSuccess(`已${actionLabel}租赁 ${leaseId}`);
    }
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
        </div>
        <div class="admin-toolbar-right">
          <label class="search-field">
            <Icon name="search" :size="16" />
            <input v-model="search" type="search" maxlength="128" placeholder="搜索租赁 / 用户 / 开发板" />
          </label>
          <label class="field filter-field">
            <span>状态</span>
            <select v-model="stateFilter">
              <option value="all">全部状态</option>
              <option value="active">占用中</option>
              <option value="releasing">释放中</option>
              <option value="released">已释放</option>
              <option value="expired">已过期</option>
              <option value="failed">失败</option>
            </select>
          </label>
        </div>
      </div>

      <div v-if="loading" class="empty-state">正在加载租赁...</div>
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
              <th class="col-actions">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(item, index) in filteredLeases" :key="item.lease.id">
              <td class="col-index">{{ index + 1 }}</td>
              <td>{{ item.lease.id }}</td>
              <td>
                <div class="table-cell-stack">
                  <div class="table-cell-stack-body">
                    <span class="table-cell-main">{{ userLabel(item.lease.user_id) }}</span>
                    <span class="table-cell-sub">{{ item.lease.user_id }}</span>
                  </div>
                </div>
              </td>
              <td>
                <div class="table-cell-stack">
                  <div class="table-cell-stack-body">
                    <span class="table-cell-main">{{ item.lease.board_id }}</span>
                    <span class="table-cell-sub">{{ item.lease.board_type }}</span>
                  </div>
                </div>
              </td>
              <td>{{ item.lease.session_id || "-" }}</td>
              <td>
                <StatusPill :tone="leaseTone(item.lease.state)" :label="leaseStateLabel(item.lease.state)" />
              </td>
              <td>
                <div class="table-cell-stack">
                  <div class="table-cell-stack-body">
                    <span class="table-cell-main">{{ formatDateTime(item.lease.starts_at) }} ~ {{ formatDateTime(item.lease.expires_at) }}</span>
                    <span class="table-cell-sub">时长 {{ formatDuration(item.lease.starts_at, item.lease.expires_at) }}</span>
                  </div>
                </div>
              </td>
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
                        :disabled="!item.lease.session_id && !item.session"
                        @click="goToSession(item)"
                      >
                        <Icon name="terminal" :size="14" />
                        转到会话
                      </button>
                      <button
                        class="action-menu-item"
                        :disabled="!canDeleteLeases"
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
</template>

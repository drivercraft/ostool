<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { useRoute, useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import type { AdminSessionResponse, AdminUserResponse, BoardConfig, SessionRecord } from "@/types/api";
import { getSessionDisplayStatus } from "@/utils/sessionStatus";
import { formatLeaseRemaining } from "@/utils/time";

const ui = useUiStore();
const auth = useAuthStore();
const route = useRoute();
const router = useRouter();
const loading = ref(true);
const boards = ref<BoardConfig[]>([]);
const users = ref<AdminUserResponse[]>([]);
const sessions = ref<AdminSessionResponse[]>([]);
const initialQuery = typeof route.query.q === "string" ? route.query.q : "";
const search = ref(initialQuery);
const stateFilter = ref<"all" | "active" | "releasing" | "released" | "expired" | "failed">("all");
const openMenuSessionId = ref<string | null>(null);
const menuPosition = ref({ top: 0, left: 0 });
const editingSession = ref<SessionRecord | null>(null);
const submitting = ref(false);
const editForm = reactive({
  client_name: "",
  failure_message: "",
});
const canDeleteSessions = computed(() => auth.hasPermission("sessions.delete"));
const canUpdateSessions = computed(() => auth.hasPermission("sessions.update"));

const boardMap = computed(() =>
  new Map(boards.value.map((board) => [board.id, board])),
);
const userMap = computed(() =>
  new Map(users.value.map((user) => [user.id, user])),
);

const filteredSessions = computed(() =>
  sessions.value.filter((item) => {
    const session = item.session;
    if (stateFilter.value !== "all" && session.state !== stateFilter.value) {
      return false;
    }
    const query = search.value.trim().toLowerCase();
    if (!query) {
      return true;
    }
    const board = boardMap.value.get(session.board_id);
    return [
      session.id,
      item.lease?.id ?? "",
      item.user_id ?? "",
      userLabel(item.user_id),
      sessionSourceIp(item),
      session.board_id,
      board?.id ?? "",
      board?.board_type ?? "",
      session.client_name ?? "",
    ].some((value) => value.toLowerCase().includes(query));
  }),
);

function userLabel(userId: string | null | undefined) {
  if (!userId) {
    return "-";
  }
  const user = userMap.value.get(userId);
  if (!user) {
    return userId;
  }
  return user.display_name || user.username;
}

function sessionSourceIp(item: AdminSessionResponse) {
  return item.source_ip || item.session.source_ip || "-";
}

function sessionDurationLabel(session: SessionRecord) {
  if (session.state === "active" || session.state === "releasing") {
    return formatLeaseRemaining(session.expires_at);
  }
  if (session.ended_at) {
    return new Date(session.ended_at).toLocaleString();
  }
  return "-";
}

function canCloseSession(session: SessionRecord) {
  return canDeleteSessions.value && session.state === "active";
}

function canDeleteSession() {
  return canDeleteSessions.value;
}

function toggleMenu(sessionId: string, event: MouseEvent) {
  if (openMenuSessionId.value === sessionId) {
    closeMenu();
    return;
  }
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  menuPosition.value = {
    top: rect.bottom + 6,
    left: Math.max(12, rect.right - 180),
  };
  openMenuSessionId.value = sessionId;
}

function closeMenu() {
  openMenuSessionId.value = null;
}

function onDocumentClick(event: MouseEvent) {
  const target = event.target as HTMLElement | null;
  if (target && !target.closest(".row-action-menu") && !target.closest(".action-menu")) {
    closeMenu();
  }
}

function goToLease(item: AdminSessionResponse) {
  if (!item.lease?.id) {
    return;
  }
  closeMenu();
  void router.push({ name: "admin-rental-lease-edit", params: { leaseId: item.lease.id } });
}

function openEdit(session: SessionRecord) {
  closeMenu();
  editingSession.value = session;
  editForm.client_name = session.client_name ?? "";
  editForm.failure_message = session.failure_message ?? "";
}

function closeEdit() {
  if (submitting.value) {
    return;
  }
  editingSession.value = null;
}

async function submitEdit() {
  if (!editingSession.value || !canUpdateSessions.value) {
    ui.setError("缺少编辑会话数据权限");
    return;
  }
  submitting.value = true;
  try {
    const updated = await api.admin.updateSession(editingSession.value.id, {
      client_name: editForm.client_name.trim() || null,
      failure_message: editForm.failure_message.trim() || null,
    });
    sessions.value = sessions.value.map((item) => (
      item.session.id === updated.session.id ? updated : item
    ));
    ui.setSuccess(`已保存会话记录 ${updated.session.id}`);
    editingSession.value = null;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    submitting.value = false;
  }
}

async function closeSessionRecord(session: SessionRecord) {
  closeMenu();
  if (!canDeleteSessions.value) {
    ui.setError("缺少删除会话数据权限");
    return;
  }
  const confirmed = await ui.confirm({
    tone: "danger",
    title: "关闭会话",
    message: `确认强制关闭会话 ${session.id} 吗？`,
    confirmLabel: "关闭",
  });
  if (!confirmed) {
    return;
  }

  try {
    await api.admin.closeSession(session.id);
    ui.setSuccess(`已发起关闭会话 ${session.id}`);
    await loadSessions();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

async function loadSessions() {
  loading.value = true;
  try {
    const [boardList, sessionList, userList] = await Promise.all([
      api.admin.listBoards(),
      api.admin.listSessions(),
      api.admin.listAdminUsers(),
    ]);
    boards.value = boardList;
    users.value = userList.users;
    sessions.value = sessionList.sessions;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

async function deleteSessionRecord(sessionId: string) {
  closeMenu();
  if (!canDeleteSessions.value) {
    ui.setError("缺少删除会话数据权限");
    return;
  }
  const confirmed = await ui.confirm({
    tone: "danger",
    title: "删除会话记录",
    message: `确认删除会话记录 ${sessionId} 吗？`,
    confirmLabel: "删除",
  });
  if (!confirmed) {
    return;
  }

  try {
    await api.admin.deleteSession(sessionId);
    ui.setSuccess(`已删除会话记录 ${sessionId}`);
    await loadSessions();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

onMounted(() => {
  document.addEventListener("click", onDocumentClick);
  ui.clearMessages();
  void loadSessions();
});

onUnmounted(() => document.removeEventListener("click", onDocumentClick));
</script>

<template>
  <section class="page-grid admin-list-page admin-list-content">
      <div class="admin-toolbar">
        <div class="admin-toolbar-left"></div>
        <div class="admin-toolbar-right">
          <label class="search-field">
            <Icon name="search" :size="16" />
            <input v-model="search" type="search" maxlength="128" placeholder="搜索会话 / 用户 / 开发板 / 源 IP" />
          </label>
          <label class="field select-field filter-field">
            <span>状态</span>
            <select v-model="stateFilter">
              <option value="all">全部状态</option>
              <option value="active">已连接</option>
              <option value="releasing">断开中</option>
              <option value="released">已断开</option>
              <option value="expired">已超时</option>
              <option value="failed">异常</option>
            </select>
          </label>
        </div>
      </div>

      <div v-if="loading" class="empty-state">正在加载会话列表...</div>
      <div v-else class="table-scroll">
        <table class="data-table">
          <thead>
            <tr>
              <th class="col-index">序号</th>
              <th>会话 ID</th>
              <th>源 IP</th>
              <th>用户</th>
              <th>开发板</th>
              <th>客户端</th>
              <th>开始时间</th>
              <th>剩余/结束时间</th>
              <th>状态</th>
              <th class="col-actions">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(item, index) in filteredSessions" :key="item.session.id">
              <td class="col-index">{{ index + 1 }}</td>
              <td>{{ item.session.id }}</td>
              <td>{{ sessionSourceIp(item) }}</td>
              <td>
                <div class="table-cell-stack">
                  <div class="table-cell-stack-body">
                    <span class="table-cell-main">{{ userLabel(item.user_id) }}</span>
                    <span class="table-cell-sub">{{ item.user_id || "-" }}</span>
                  </div>
                </div>
              </td>
              <td>
                <div class="table-cell-stack">
                  <div class="table-cell-stack-body">
                    <span class="table-cell-main">{{ boardMap.get(item.session.board_id)?.id || item.session.board_id }}</span>
                    <span class="table-cell-sub">{{ boardMap.get(item.session.board_id)?.board_type || "-" }}</span>
                  </div>
                </div>
              </td>
              <td>{{ item.session.client_name || "-" }}</td>
              <td>{{ new Date(item.session.created_at).toLocaleString() }}</td>
              <td>{{ sessionDurationLabel(item.session) }}</td>
              <td>
                <StatusPill
                  :tone="getSessionDisplayStatus(item.session.state).tone"
                  :label="getSessionDisplayStatus(item.session.state).label"
                />
              </td>
              <td class="col-actions">
                <div class="row-actions">
                  <button
                    class="btn-icon-only"
                    title="编辑"
                    :disabled="!canUpdateSessions"
                    @click="openEdit(item.session)"
                  >
                    <Icon name="edit" :size="16" />
                  </button>
                  <button
                    class="btn-icon-only"
                    title="关闭"
                    :disabled="!canCloseSession(item.session)"
                    @click="closeSessionRecord(item.session)"
                  >
                    <Icon name="ban" :size="16" />
                  </button>
                  <div class="row-action-menu">
                    <button
                      class="btn-icon-only"
                      title="更多"
                      :aria-expanded="openMenuSessionId === item.session.id"
                      @click.stop="toggleMenu(item.session.id, $event)"
                    >
                      <Icon name="more-vertical" :size="16" />
                    </button>
                  </div>
                  <Teleport to="body">
                    <div
                      v-if="openMenuSessionId === item.session.id"
                      class="action-menu action-menu--floating"
                      :style="{ top: `${menuPosition.top}px`, left: `${menuPosition.left}px` }"
                    >
                      <button
                        class="action-menu-item"
                        :disabled="!item.lease"
                        @click="goToLease(item)"
                      >
                        <Icon name="clipboard" :size="14" />
                        转到租赁
                      </button>
                      <button
                        class="action-menu-item"
                        :disabled="!canDeleteSession()"
                        @click="deleteSessionRecord(item.session.id)"
                      >
                        <Icon name="trash" :size="14" />
                        删除记录
                      </button>
                    </div>
                  </Teleport>
                </div>
              </td>
            </tr>
            <tr v-if="filteredSessions.length === 0" class="table-empty-row">
              <td colspan="10">
                <div class="empty-state">暂无会话数据</div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div v-if="!loading" class="table-statusbar">
        <span>{{ filteredSessions.length === 0 ? "暂无分页" : "第 1 / 共 1 页" }}</span>
        <span>本页 {{ filteredSessions.length }} 条</span>
        <span>筛选后 {{ filteredSessions.length }} 条 / 共 {{ sessions.length }} 条</span>
      </div>
    <div v-if="editingSession" class="modal-overlay">
      <div class="modal-card modal-card--narrow">
        <header class="modal-header">
          <h3>编辑会话记录</h3>
          <button class="btn-icon-only modal-close-button" title="关闭" @click="closeEdit">×</button>
        </header>

        <form class="modal-form" @submit.prevent="submitEdit">
          <div class="modal-body">
            <label class="field">
              <span>客户端名称</span>
              <input
                v-model="editForm.client_name"
                maxlength="128"
                placeholder="客户端名称"
              />
            </label>
            <label class="field">
              <span>异常说明</span>
              <textarea
                v-model="editForm.failure_message"
                maxlength="500"
                rows="4"
                placeholder="可填写异常或备注信息"
              />
            </label>
          </div>

          <div class="modal-actions toolbar-actions">
            <button type="submit" class="btn btn-primary" :disabled="submitting">
              {{ submitting ? "保存中..." : "保存" }}
            </button>
            <button type="button" class="btn btn-ghost" :disabled="submitting" @click="closeEdit">取消</button>
          </div>
        </form>
      </div>
    </div>
  </section>
</template>

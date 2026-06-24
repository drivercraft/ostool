<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRoute } from "vue-router";

import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import type { AdminSessionResponse, AdminUserResponse, BoardConfig, SessionRecord } from "@/types/api";
import { formatLeaseRemaining } from "@/utils/time";

const ui = useUiStore();
const auth = useAuthStore();
const route = useRoute();
const loading = ref(true);
const boards = ref<BoardConfig[]>([]);
const users = ref<AdminUserResponse[]>([]);
const sessions = ref<AdminSessionResponse[]>([]);
const initialQuery = typeof route.query.q === "string" ? route.query.q : "";
const search = ref(initialQuery);
const stateFilter = ref<"all" | "active" | "releasing" | "released" | "expired" | "failed">("all");
const canDeleteRentals = computed(() => auth.hasPermission("rentals.delete"));

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

function sessionStatus(session: SessionRecord) {
  const labels = {
    active: { tone: "warn" as const, label: "占用中" },
    releasing: { tone: "neutral" as const, label: "释放中" },
    released: { tone: "good" as const, label: "已释放" },
    expired: { tone: "neutral" as const, label: "已过期" },
    failed: { tone: "danger" as const, label: "失败" },
  };
  return labels[session.state] ?? { tone: "neutral" as const, label: session.state };
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

function canDeleteSession(session: SessionRecord) {
  return canDeleteRentals.value && session.state !== "releasing";
}

async function loadSessions() {
  loading.value = true;
  try {
    const [boardList, sessionList, userList] = await Promise.all([
      api.listBoards(),
      api.listSessions(),
      api.listAdminUsers(),
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
  if (!canDeleteRentals.value) {
    ui.setError("缺少删除租赁数据权限");
    return;
  }
  const confirmed = await ui.confirm({
    tone: "danger",
    title: "删除会话租约",
    message: `确认删除会话租约 ${sessionId} 吗？`,
    confirmLabel: "删除",
  });
  if (!confirmed) {
    return;
  }

  try {
    await api.deleteSession(sessionId);
    ui.setSuccess(`已删除会话租约 ${sessionId}`);
    await loadSessions();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

onMounted(() => {
  ui.clearMessages();
  void loadSessions();
});
</script>

<template>
  <section class="page-grid admin-list-page">
    <div class="panel admin-table-panel">
      <div class="admin-toolbar">
        <div class="admin-toolbar-left">
          <button class="btn btn-ghost btn-sm" @click="loadSessions">刷新</button>
        </div>
        <div class="admin-toolbar-right">
          <label class="search-field">
            <Icon name="search" :size="16" />
            <input v-model="search" type="search" placeholder="搜索会话 / 用户 / 开发板 / 源 IP" />
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
                <strong>{{ userLabel(item.user_id) }}</strong>
                <div>{{ item.user_id || "-" }}</div>
              </td>
              <td>
                <span>{{ boardMap.get(item.session.board_id)?.id || item.session.board_id }}</span>
                <div class="muted">{{ boardMap.get(item.session.board_id)?.board_type || "-" }}</div>
              </td>
              <td>{{ item.session.client_name || "-" }}</td>
              <td>{{ new Date(item.session.created_at).toLocaleString() }}</td>
              <td>{{ sessionDurationLabel(item.session) }}</td>
              <td>
                <StatusPill
                  :tone="sessionStatus(item.session).tone"
                  :label="sessionStatus(item.session).label"
                />
              </td>
              <td class="col-actions">
                <div class="row-actions">
                  <button
                    class="btn-icon-only"
                    title="删除"
                    :disabled="!canDeleteSession(item.session)"
                    @click="deleteSessionRecord(item.session.id)"
                  >
                    <Icon name="trash" :size="16" />
                  </button>
                </div>
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
    </div>
  </section>
</template>

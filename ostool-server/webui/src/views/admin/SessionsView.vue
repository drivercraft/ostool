<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRoute } from "vue-router";

import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { BoardConfig, Session } from "@/types/api";
import { formatLeaseRemaining } from "@/utils/time";

const ui = useUiStore();
const route = useRoute();
const loading = ref(true);
const boards = ref<BoardConfig[]>([]);
const sessions = ref<Session[]>([]);
const initialQuery = typeof route.query.q === "string" ? route.query.q : "";
const search = ref(initialQuery);
const stateFilter = ref<"all" | "active" | "releasing">("all");

const boardMap = computed(() =>
  new Map(boards.value.map((board) => [board.id, board])),
);

const filteredSessions = computed(() =>
  sessions.value.filter((session) => {
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
      session.board_id,
      board?.id ?? "",
      board?.board_type ?? "",
      session.client_name ?? "",
    ].some((value) => value.toLowerCase().includes(query));
  }),
);

function sessionStatus(session: Session) {
  if (session.state === "releasing") {
    return {
      tone: "neutral" as const,
      label: "释放中",
    };
  }

  return {
    tone: "warn" as const,
    label: "占用中",
  };
}

async function loadSessions() {
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

async function releaseSession(sessionId: string) {
  const confirmed = await ui.confirm({
    tone: "danger",
    title: "释放会话",
    message: `确认释放会话 ${sessionId} 吗？`,
    confirmLabel: "释放",
  });
  if (!confirmed) {
    return;
  }

  try {
    await api.deleteSession(sessionId);
    ui.setSuccess(`已发起释放会话 ${sessionId}`);
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
  <section class="page-grid">
    <div class="admin-toolbar">
      <div class="admin-toolbar-left">
        <button class="btn btn-ghost btn-sm" @click="loadSessions">刷新</button>
      </div>
      <div class="admin-toolbar-right">
        <label class="search-field">
          <Icon name="search" :size="16" />
          <input v-model="search" placeholder="搜索会话 / 开发板 / 客户端" />
        </label>
        <label class="field filter-field">
          <span>状态</span>
          <select v-model="stateFilter">
            <option value="all">全部状态</option>
            <option value="active">占用中</option>
            <option value="releasing">释放中</option>
          </select>
        </label>
      </div>
    </div>

    <div class="panel admin-table-panel">
      <div v-if="loading" class="empty-state">正在加载会话列表...</div>
      <div v-else-if="sessions.length === 0" class="empty-state">当前没有活跃会话。</div>
      <div v-else-if="filteredSessions.length === 0" class="empty-state">没有符合条件的会话。</div>
      <div v-else class="table-scroll">
        <table class="data-table">
          <thead>
            <tr>
              <th class="col-index">序号</th>
              <th>Session ID</th>
              <th>开发板</th>
              <th>客户端</th>
              <th>创建时间</th>
              <th>剩余租约</th>
              <th>状态</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(session, index) in filteredSessions" :key="session.id">
              <td class="col-index">{{ index + 1 }}</td>
              <td><code>{{ session.id }}</code></td>
              <td>
                {{ boardMap.get(session.board_id)?.id || session.board_id }}
              </td>
              <td>{{ session.client_name || "-" }}</td>
              <td>{{ new Date(session.created_at).toLocaleString() }}</td>
              <td>{{ formatLeaseRemaining(session.expires_at) }}</td>
              <td>
                <StatusPill
                  :tone="sessionStatus(session).tone"
                  :label="sessionStatus(session).label"
                />
              </td>
              <td>
                <button
                  class="btn btn-danger btn-sm"
                  :disabled="session.state === 'releasing'"
                  @click="releaseSession(session.id)"
                >
                  强制释放
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </section>
</template>

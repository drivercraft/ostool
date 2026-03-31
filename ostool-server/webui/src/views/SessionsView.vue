<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api/client";
import { useUiStore } from "@/stores/ui";
import type { BoardConfig, Session } from "@/types/api";
import { formatLeaseRemaining } from "@/utils/time";

const ui = useUiStore();
const loading = ref(true);
const boards = ref<BoardConfig[]>([]);
const sessions = ref<Session[]>([]);

const boardMap = computed(() =>
  new Map(boards.value.map((board) => [board.id, board])),
);

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
  if (!window.confirm(`确认释放会话 ${sessionId} 吗？`)) {
    return;
  }

  try {
    await api.deleteSession(sessionId);
    ui.setSuccess(`已释放会话 ${sessionId}`);
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
    <div class="panel">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">租约与释放</p>
          <h3>当前活跃会话</h3>
        </div>
        <button class="ghost-button" @click="loadSessions">刷新</button>
      </div>

      <div v-if="loading" class="empty-state">正在加载会话列表...</div>
      <div v-else-if="sessions.length === 0" class="empty-state">当前没有活跃会话。</div>
      <table v-else class="data-table">
        <thead>
          <tr>
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
          <tr v-for="session in sessions" :key="session.id">
            <td><code>{{ session.id }}</code></td>
            <td>
              {{ boardMap.get(session.board_id)?.id || session.board_id }}
            </td>
            <td>{{ session.client_name || "-" }}</td>
            <td>{{ new Date(session.created_at).toLocaleString() }}</td>
            <td>{{ formatLeaseRemaining(session.expires_at) }}</td>
            <td>
              <StatusPill tone="warn" label="占用中" />
            </td>
            <td>
              <button class="inline-link danger-link" @click="releaseSession(session.id)">
                强制释放
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>

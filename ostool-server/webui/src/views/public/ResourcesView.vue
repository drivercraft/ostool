<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RouterLink } from "vue-router";

import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import type { BoardTypeSummary } from "@/types/api";

const ui = useUiStore();
const auth = useAuthStore();
const loading = ref(true);
const failed = ref(false);
const boardTypes = ref<BoardTypeSummary[]>([]);
const search = ref("");
const availabilityFilter = ref<"all" | "available" | "unavailable">("all");

async function loadBoardTypes() {
  loading.value = true;
  failed.value = false;
  try {
    boardTypes.value = await api.listBoardTypes();
  } catch (error) {
    failed.value = true;
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

const filteredBoardTypes = computed(() =>
  boardTypes.value.filter((item) => {
    if (search.value) {
      const query = search.value.toLowerCase();
      const haystack = [item.board_type, ...item.tags].join(" ").toLowerCase();
      if (!haystack.includes(query)) {
        return false;
      }
    }
    if (availabilityFilter.value === "available" && item.available === 0) {
      return false;
    }
    if (availabilityFilter.value === "unavailable" && item.available > 0) {
      return false;
    }
    return true;
  }),
);

const totals = computed(() => {
  const total = boardTypes.value.reduce((sum, item) => sum + item.total, 0);
  const available = boardTypes.value.reduce((sum, item) => sum + item.available, 0);
  return { total, available, leased: total - available };
});

onMounted(() => {
  ui.clearMessages();
  void loadBoardTypes();
});
</script>

<template>
  <div class="page-body public-page-body">
    <header class="public-page-header">
      <p class="eyebrow">资源总览</p>
      <h2>可租赁的开发板资源</h2>
      <p class="public-page-subtitle">
        下方为当前在管的全部开发板型号。登录后可在用户控制台中创建会话并申请使用。
      </p>
    </header>

    <section class="stats-strip" v-if="!loading && !failed">
      <div class="stats-chip">
        <span class="stats-num">{{ boardTypes.length }}</span>
        <span class="stats-label">型号</span>
      </div>
      <div class="stats-chip">
        <span class="stats-num">{{ totals.available }}</span>
        <span class="stats-label">当前可用</span>
      </div>
      <div class="stats-chip">
        <span class="stats-num">{{ totals.leased }}</span>
        <span class="stats-label">使用中</span>
      </div>
      <div class="stats-chip">
        <span class="stats-num">{{ totals.total }}</span>
        <span class="stats-label">在管总数</span>
      </div>
    </section>

    <section class="resource-toolbar">
      <div class="panel-heading compact resource-toolbar-head">
        <div>
          <p class="eyebrow">筛选</p>
          <h3>按需筛选型号</h3>
        </div>
        <div class="toolbar-actions">
          <button class="ghost-button compact-button" @click="loadBoardTypes">
            <Icon name="refresh" :size="14" class="btn-icon" />
            刷新
          </button>
        </div>
      </div>

      <div class="filter-bar">
        <label class="field filter-field">
          <span>关键字</span>
          <input
            v-model="search"
            placeholder="按型号或标签搜索，例如 rk3568、lab..."
          />
        </label>
        <label class="field filter-field">
          <span>可用状态</span>
          <select v-model="availabilityFilter">
            <option value="all">全部</option>
            <option value="available">仅显示可用</option>
            <option value="unavailable">仅显示已满</option>
          </select>
        </label>
      </div>
    </section>

    <div v-if="loading" class="empty-state">
      <div class="empty-state-icon">&#9641;</div>
      正在加载开发板资源...
    </div>
    <div v-else-if="failed" class="empty-state">
      <div class="empty-state-icon">&#9888;</div>
      加载失败，请稍后重试。
      <button class="inline-link" @click="loadBoardTypes">重新加载</button>
    </div>
    <div v-else-if="filteredBoardTypes.length === 0" class="empty-state">
      <div class="empty-state-icon">&#9641;</div>
      当前没有符合筛选条件的开发板型号。
    </div>

    <div v-else class="resource-card-grid">
      <article
        v-for="board in filteredBoardTypes"
        :key="board.board_type"
        class="resource-card"
      >
        <div class="resource-card-header">
          <div class="resource-card-title">
            <span class="resource-card-icon"><Icon name="cpu-board" :size="22" /></span>
            <code class="resource-card-type">{{ board.board_type }}</code>
          </div>
          <StatusPill
            :tone="board.available > 0 ? 'good' : 'neutral'"
            :label="board.available > 0 ? '可租赁' : '已租满'"
          />
        </div>
        <div class="resource-card-tags">
          <span v-for="tag in board.tags" :key="tag" class="tag-chip">{{ tag }}</span>
          <span v-if="board.tags.length === 0" class="resource-card-empty">无标签</span>
        </div>
        <dl class="resource-card-meta">
          <div>
            <dt>总数</dt>
            <dd>{{ board.total }}</dd>
          </div>
          <div>
            <dt>可用</dt>
            <dd>{{ board.available }}</dd>
          </div>
          <div>
            <dt>使用中</dt>
            <dd>{{ board.total - board.available }}</dd>
          </div>
        </dl>
        <div class="resource-card-actions">
          <RouterLink
            v-if="auth.isAuthenticated"
            class="primary-button compact-button"
            :class="{ 'is-disabled': board.available === 0 }"
            :to="board.available > 0 ? '/dashboard?action=lease&board_type=' + encodeURIComponent(board.board_type) : '/dashboard'"
          >
            {{ board.available > 0 ? "去申请会话" : "暂无空闲" }}
            <Icon v-if="board.available > 0" name="arrow-right" :size="14" class="btn-icon" />
          </RouterLink>
          <RouterLink v-else class="ghost-button compact-button" to="/login">
            登录后申请
            <Icon name="login" :size="14" class="btn-icon" />
          </RouterLink>
        </div>
      </article>
    </div>
  </div>
</template>

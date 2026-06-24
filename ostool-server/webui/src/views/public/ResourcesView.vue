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
const sortKey = ref<"name" | "available" | "total">("name");
const viewMode = ref<"grid" | "list">("grid");

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

const filteredBoardTypes = computed(() => {
  const list = boardTypes.value.filter((item) => {
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
  });
  const sorted = [...list];
  sorted.sort((a, b) => {
    if (sortKey.value === "available") return b.available - a.available;
    if (sortKey.value === "total") return b.total - a.total;
    return a.board_type.localeCompare(b.board_type);
  });
  return sorted;
});

const totals = computed(() => {
  const total = boardTypes.value.reduce((sum, item) => sum + item.total, 0);
  const available = boardTypes.value.reduce((sum, item) => sum + item.available, 0);
  const leased = total - available;
  const availabilityRate = total ? Math.round((available / total) * 100) : 0;
  return { total, available, leased, availabilityRate };
});

function availabilityPercent(board: BoardTypeSummary): number {
  if (board.total === 0) return 0;
  return Math.round((board.available / board.total) * 100);
}

function leaseCreateTarget(board: BoardTypeSummary) {
  return {
    name: "user-lease-new",
    query: { board_type: board.board_type },
  };
}

function loginTarget(board: BoardTypeSummary) {
  return {
    name: "login",
    query: {
      next: `/leases/new?board_type=${encodeURIComponent(board.board_type)}`,
    },
  };
}

onMounted(() => {
  ui.clearMessages();
  void loadBoardTypes();
});
</script>

<template>
  <div class="page-body public-page-body resources-page">
    <section class="resources-hero">
      <div class="resources-hero-copy">
        <h2>可租赁的开发板资源</h2>
        <p class="public-page-subtitle">
          下方为当前在管的全部开发板型号。登录后可在用户控制台中创建会话并申请使用。
        </p>
      </div>
      <section class="stats-strip resources-stats" v-if="!loading && !failed">
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
    </section>

    <section class="admin-toolbar">
      <div class="admin-toolbar-left">
        <label class="search-field">
          <Icon name="search" :size="16" />
          <input
            v-model="search"
            type="search"
            maxlength="128"
            placeholder="搜索型号或标签，例如 rk3568、lab..."
          />
        </label>
        <label class="field filter-field">
          <span>可用状态</span>
          <select v-model="availabilityFilter" aria-label="可用状态">
            <option value="all">全部状态</option>
            <option value="available">仅显示可用</option>
            <option value="unavailable">仅显示已满</option>
          </select>
        </label>
        <label class="field filter-field">
          <span>排序方式</span>
          <select v-model="sortKey" aria-label="排序方式">
            <option value="name">按型号排序</option>
            <option value="available">可用数优先</option>
            <option value="total">总数优先</option>
          </select>
        </label>
      </div>
      <div class="admin-toolbar-right">
        <div class="view-toggle" role="group" aria-label="排列方式">
          <button
            type="button"
            :class="{ 'is-active': viewMode === 'grid' }"
            @click="viewMode = 'grid'"
          >
            <Icon name="cube" :size="15" /> 卡片
          </button>
          <button
            type="button"
            :class="{ 'is-active': viewMode === 'list' }"
            @click="viewMode = 'list'"
          >
            <Icon name="clipboard" :size="15" /> 列表
          </button>
        </div>
        <button class="btn btn-ghost btn-sm" @click="loadBoardTypes">
          <Icon name="refresh" :size="14" class="btn-icon" />
          刷新
        </button>
      </div>
    </section>

    <div v-if="!loading && !failed" class="resource-results-summary">
      <span>显示 {{ filteredBoardTypes.length }} / {{ boardTypes.length }} 个型号</span>
      <span>整体可用率 {{ totals.availabilityRate }}%</span>
    </div>

    <div v-if="loading" class="empty-state">
      <div class="empty-state-icon">&#9641;</div>
      正在加载开发板资源...
    </div>
    <div v-else-if="failed" class="empty-state">
      <div class="empty-state-icon">&#9888;</div>
      加载失败，请稍后重试。
      <button class="btn btn-ghost btn-sm" @click="loadBoardTypes">重新加载</button>
    </div>
    <div v-else-if="filteredBoardTypes.length === 0" class="empty-state">
      <div class="empty-state-icon">&#9641;</div>
      当前没有符合筛选条件的开发板型号。
    </div>

    <div v-else-if="viewMode === 'grid'" class="board-grid">
      <article
        v-for="board in filteredBoardTypes"
        :key="board.board_type"
        class="resource-card card--hover"
      >
        <div class="resource-card-header">
          <div class="resource-card-id">
            <span class="resource-card-icon"><Icon name="cpu-board" :size="20" /></span>
            <div>
              <h3>{{ board.board_type }}</h3>
              <span class="resource-card-meta">Board type</span>
            </div>
          </div>
          <StatusPill
            :tone="board.available > 0 ? 'good' : 'neutral'"
            :label="board.available > 0 ? '可租赁' : '已租满'"
          />
        </div>
        <div class="resource-card-tags">
          <span v-for="tag in board.tags" :key="tag" class="resource-tag">{{ tag }}</span>
          <span v-if="board.tags.length === 0" class="resource-tag resource-tag--muted">无标签</span>
        </div>
        <div class="resource-availability">
          <div>
            <span class="resource-availability-label">可用容量</span>
            <strong>{{ availabilityPercent(board) }}%</strong>
          </div>
          <span>{{ board.available }} / {{ board.total }} idle</span>
        </div>
        <div class="resource-card-bar">
          <span :style="{ width: availabilityPercent(board) + '%' }"></span>
        </div>
        <dl class="resource-card-stats">
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
            class="btn btn-primary btn-sm"
            :class="{ 'is-disabled': board.available === 0 }"
            :to="board.available > 0 ? leaseCreateTarget(board) : '/dashboard'"
          >
            {{ board.available > 0 ? "申请租赁" : "暂无空闲" }}
            <Icon v-if="board.available > 0" name="arrow-right" :size="14" class="btn-icon" />
          </RouterLink>
          <RouterLink v-else class="btn btn-ghost btn-sm" :to="loginTarget(board)">
            登录后申请
            <Icon name="login" :size="14" class="btn-icon" />
          </RouterLink>
        </div>
      </article>
    </div>

    <div v-else class="board-list">
      <div
        v-for="board in filteredBoardTypes"
        :key="board.board_type"
        class="board-row"
      >
        <div class="resource-card-id">
          <span class="resource-card-icon"><Icon name="cpu-board" :size="18" /></span>
          <div>
            <strong>{{ board.board_type }}</strong>
            <span class="resource-card-meta">{{ board.tags.length ? board.tags.join(" · ") : "无标签" }}</span>
          </div>
        </div>
        <div>
          <span class="row-label">可用 / 总数</span>
          <div><strong>{{ board.available }}</strong> / {{ board.total }}</div>
        </div>
        <div>
          <span class="row-label">可用率</span>
          <div>{{ availabilityPercent(board) }}%</div>
        </div>
        <div>
          <span class="row-label">使用中</span>
          <div>{{ board.total - board.available }}</div>
        </div>
        <div>
          <StatusPill
            :tone="board.available > 0 ? 'good' : 'neutral'"
            :label="board.available > 0 ? '可租赁' : '已租满'"
          />
        </div>
        <div class="resource-card-actions">
          <RouterLink
            v-if="auth.isAuthenticated"
            class="btn btn-primary btn-sm"
            :class="{ 'is-disabled': board.available === 0 }"
            :to="board.available > 0 ? leaseCreateTarget(board) : '/dashboard'"
          >
            {{ board.available > 0 ? "申请租赁" : "已满" }}
          </RouterLink>
          <RouterLink v-else class="btn btn-ghost btn-sm" :to="loginTarget(board)">登录</RouterLink>
        </div>
      </div>
    </div>
  </div>
</template>

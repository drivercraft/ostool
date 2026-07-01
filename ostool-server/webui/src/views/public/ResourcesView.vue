<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RouterLink } from "vue-router";

import Icon, { type IconName } from "@/components/Icon.vue";
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
    boardTypes.value = await api.public.listBoardTypes();
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

interface StatCard {
  key: string;
  label: string;
  value: number | string;
  suffix?: string;
  icon: IconName;
  tone: "brand" | "success" | "violet" | "sky";
}

const heroStats = computed<StatCard[]>(() => [
  { key: "models", label: "开发板型号", value: boardTypes.value.length, suffix: " 款", icon: "cpu-board", tone: "brand" },
  { key: "available", label: "当前可用", value: totals.value.available, suffix: " 块", icon: "check", tone: "success" },
  { key: "leased", label: "使用中", value: totals.value.leased, suffix: " 块", icon: "pulse", tone: "violet" },
  { key: "total", label: "在管总数", value: totals.value.total, suffix: " 块", icon: "cube", tone: "sky" },
]);

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
        <p class="eyebrow">资源总览</p>
        <h2>可租赁的开发板资源</h2>
        <p class="public-page-subtitle">
          下方为平台当前纳管的全部开发板型号。登录后可在用户控制台中创建租赁，并按时间段预约设备。
        </p>
      </div>
      <section class="resources-stats" v-if="!loading && !failed">
        <article
          v-for="stat in heroStats"
          :key="stat.key"
          class="resource-stat"
          :data-tone="stat.tone"
        >
          <span class="resource-stat-icon"><Icon :name="stat.icon" :size="20" /></span>
          <div class="resource-stat-body">
            <div class="resource-stat-value">
              {{ stat.value }}<span v-if="stat.suffix" class="resource-stat-suffix">{{ stat.suffix }}</span>
            </div>
            <div class="resource-stat-label">{{ stat.label }}</div>
          </div>
        </article>
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
          <span>当前空闲</span>
          <select v-model="availabilityFilter" aria-label="当前空闲">
            <option value="all">全部状态</option>
            <option value="available">当前有空闲</option>
            <option value="unavailable">当前无空闲</option>
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
      </div>
    </section>

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
      暂无开发板资源数据
    </div>

    <div v-else-if="viewMode === 'grid'" class="board-grid">
      <article
        v-for="board in filteredBoardTypes"
        :key="board.board_type"
        class="resource-card card--hover"
        :class="{ 'resource-card--empty': board.available === 0 }"
      >
        <header class="resource-card-header">
          <div class="resource-card-id">
            <span class="resource-card-icon"><Icon name="cpu-board" :size="22" /></span>
            <div>
              <h3>{{ board.board_type }}</h3>
              <span class="resource-card-subtitle">
                {{ board.total }} 块在管 · {{ board.available }} 可用
              </span>
            </div>
          </div>
          <StatusPill
            :tone="board.available > 0 ? 'good' : 'neutral'"
            :label="board.available > 0 ? '可租赁' : '已租满'"
          />
        </header>

        <div v-if="board.tags.length > 0" class="resource-card-tags">
          <span v-for="tag in board.tags" :key="tag" class="resource-tag">{{ tag }}</span>
        </div>

        <div class="resource-capacity">
          <div class="resource-capacity-head">
            <span class="resource-capacity-label">空闲容量</span>
            <span class="resource-capacity-num">
              <strong>{{ board.available }}</strong> / {{ board.total }}
            </span>
          </div>
          <div class="resource-card-bar">
            <span :style="{ width: availabilityPercent(board) + '%' }"></span>
          </div>
        </div>

        <footer class="resource-card-actions">
          <RouterLink
            v-if="auth.isAuthenticated"
            class="btn btn-primary"
            :to="leaseCreateTarget(board)"
          >
            <Icon name="arrow-right" :size="15" class="btn-icon" />
            申请租赁
          </RouterLink>
          <RouterLink v-else class="btn btn-ghost" :to="loginTarget(board)">
            <Icon name="login" :size="15" class="btn-icon" />
            登录后申请
          </RouterLink>
        </footer>
      </article>
    </div>

    <div v-else class="board-list">
      <div class="board-row board-row-head">
        <div class="row-label">型号 / 标签</div>
        <div class="row-label">可用 / 总数</div>
        <div class="row-label">可用率</div>
        <div class="row-label">状态</div>
        <div class="row-label text-right">操作</div>
      </div>
      <div
        v-for="board in filteredBoardTypes"
        :key="board.board_type"
        class="board-row"
      >
        <div class="resource-card-id">
          <span class="resource-card-icon resource-card-icon--sm"><Icon name="cpu-board" :size="18" /></span>
          <div>
            <strong>{{ board.board_type }}</strong>
            <span class="resource-card-tags-inline">
              <span v-if="board.tags.length === 0" class="resource-tag resource-tag--muted">无标签</span>
              <span v-for="tag in board.tags" :key="tag" class="resource-tag">{{ tag }}</span>
            </span>
          </div>
        </div>
        <div>
          <strong class="row-num">{{ board.available }}</strong>
          <span class="row-sep"> / </span>{{ board.total }}
        </div>
        <div>
          <div class="row-rate">
            <div class="resource-card-bar resource-card-bar--sm">
              <span :style="{ width: availabilityPercent(board) + '%' }"></span>
            </div>
            <span>{{ availabilityPercent(board) }}%</span>
          </div>
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
            :to="leaseCreateTarget(board)"
          >
            申请租赁
          </RouterLink>
          <RouterLink v-else class="btn btn-ghost btn-sm" :to="loginTarget(board)">登录</RouterLink>
        </div>
      </div>
    </div>
  </div>
</template>

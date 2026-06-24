<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RouterLink, useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import type { LeaseResponse } from "@/types/api";
import { formatLeaseTime, remainingLeaseLabel, useUserLeases } from "./useUserLeases";

type CalendarViewMode = "hour" | "day" | "month" | "year";
type CalendarSlot = {
  key: string;
  label: string;
  caption: string;
  startIso: string;
  endIso: string;
  leases: LeaseResponse[];
};

const ui = useUiStore();
const auth = useAuthStore();
const router = useRouter();
const calendarView = ref<CalendarViewMode>("month");
const calendarCursorIso = ref(new Date().toISOString());
const {
  loading,
  activeLeases,
  activeSessions,
  loadLeases,
  releaseLease,
} = useUserLeases();

const calendarAnchor = computed(() => {
  const date = new Date(calendarCursorIso.value);
  return Number.isNaN(date.getTime()) ? new Date() : date;
});
const calendarSlots = computed(() => {
  if (calendarView.value === "hour") {
    return buildHourSlots(calendarAnchor.value);
  }
  if (calendarView.value === "day") {
    return buildDaySlots(calendarAnchor.value);
  }
  if (calendarView.value === "year") {
    return buildYearSlots(calendarAnchor.value);
  }
  return buildMonthSlots(calendarAnchor.value);
});
const calendarPeriodLabel = computed(() => {
  const first = calendarSlots.value[0];
  const last = calendarSlots.value[calendarSlots.value.length - 1];
  if (calendarView.value === "hour") {
    return `${formatDateTime(first.startIso)} ~ ${formatDateTime(last.endIso)}`;
  }
  if (calendarView.value === "day") {
    return `${formatDate(first.startIso)} ~ ${formatDate(last.startIso)}`;
  }
  if (calendarView.value === "year") {
    return `${new Date(first.startIso).getFullYear()} ~ ${new Date(last.startIso).getFullYear()}`;
  }
  return `${formatMonth(first.startIso)} ~ ${formatMonth(last.startIso)}`;
});

function windowsOverlap(startA: string, endA: string, startB: string, endB: string) {
  if (!startA || !endA || !startB || !endB) {
    return false;
  }
  return new Date(startA).getTime() < new Date(endB).getTime()
    && new Date(endA).getTime() > new Date(startB).getTime();
}

function leasesForRange(startIso: string, endIso: string) {
  return activeLeases.value.filter((item) =>
    windowsOverlap(item.lease.starts_at, item.lease.expires_at, startIso, endIso),
  );
}

function makeCalendarSlot(label: string, caption: string, start: Date, end: Date): CalendarSlot {
  const startIso = start.toISOString();
  const endIso = end.toISOString();
  return {
    key: `${startIso}-${endIso}`,
    label,
    caption,
    startIso,
    endIso,
    leases: leasesForRange(startIso, endIso),
  };
}

function buildHourSlots(anchor: Date) {
  const start = new Date(anchor);
  start.setMinutes(0, 0, 0);
  start.setHours(start.getHours() - 12);
  return Array.from({ length: 24 }, (_, index) => {
    const slotStart = new Date(start);
    slotStart.setHours(start.getHours() + index);
    const slotEnd = new Date(slotStart);
    slotEnd.setHours(slotStart.getHours() + 1);
    return makeCalendarSlot(
      slotStart.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false }),
      formatShortDate(slotStart.toISOString()),
      slotStart,
      slotEnd,
    );
  });
}

function buildDaySlots(anchor: Date) {
  const start = new Date(anchor);
  start.setHours(0, 0, 0, 0);
  start.setMonth(start.getMonth() - 1);
  return Array.from({ length: 63 }, (_, index) => {
    const slotStart = new Date(start);
    slotStart.setDate(start.getDate() + index);
    const slotEnd = new Date(slotStart);
    slotEnd.setDate(slotStart.getDate() + 1);
    return makeCalendarSlot(formatShortDate(slotStart.toISOString()), "", slotStart, slotEnd);
  });
}

function buildMonthSlots(anchor: Date) {
  const start = new Date(anchor.getFullYear(), anchor.getMonth() - 12, 1);
  return Array.from({ length: 25 }, (_, index) => {
    const slotStart = new Date(start.getFullYear(), start.getMonth() + index, 1);
    const slotEnd = new Date(slotStart.getFullYear(), slotStart.getMonth() + 1, 1);
    return makeCalendarSlot(
      slotStart.toLocaleDateString("zh-CN", { month: "long" }),
      `${slotStart.getFullYear()}`,
      slotStart,
      slotEnd,
    );
  });
}

function buildYearSlots(anchor: Date) {
  const startYear = anchor.getFullYear() - 12;
  return Array.from({ length: 25 }, (_, index) => {
    const year = startYear + index;
    return makeCalendarSlot(`${year}`, "全年", new Date(year, 0, 1), new Date(year + 1, 0, 1));
  });
}

function moveCalendar(step: number) {
  const next = new Date(calendarAnchor.value);
  if (calendarView.value === "hour") {
    next.setHours(next.getHours() + step * 24);
  } else if (calendarView.value === "day") {
    next.setMonth(next.getMonth() + step);
  } else if (calendarView.value === "year") {
    next.setFullYear(next.getFullYear() + step * 12);
  } else {
    next.setMonth(next.getMonth() + step * 12);
  }
  calendarCursorIso.value = next.toISOString();
}

function focusNow() {
  calendarCursorIso.value = new Date().toISOString();
}

function formatTime(value: string) {
  return new Date(value).toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

function formatDateTime(value: string) {
  return new Date(value).toLocaleString("zh-CN", { hour12: false });
}

function formatDate(value: string) {
  return new Date(value).toLocaleDateString("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  });
}

function formatShortDate(value: string) {
  return new Date(value).toLocaleDateString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
  });
}

function formatMonth(value: string) {
  return new Date(value).toLocaleDateString("zh-CN", {
    year: "numeric",
    month: "long",
  });
}

function eventLabelForSlot(item: LeaseResponse, slot: CalendarSlot) {
  const slotStart = new Date(slot.startIso).getTime();
  const slotEnd = new Date(slot.endIso).getTime();
  const start = new Date(item.lease.starts_at).getTime();
  const end = new Date(item.lease.expires_at).getTime();
  if (start <= slotStart && end >= slotEnd) {
    return calendarView.value === "hour"
      ? "整点租赁"
      : calendarView.value === "day"
        ? "全天租赁"
        : calendarView.value === "year"
          ? "整年租赁"
          : "整月租赁";
  }
  if (start < slotStart) {
    return `截至 ${formatTime(item.lease.expires_at)}`;
  }
  if (end > slotEnd) {
    return `${formatTime(item.lease.starts_at)} 起`;
  }
  return `${formatTime(item.lease.starts_at)} ~ ${formatTime(item.lease.expires_at)}`;
}

onMounted(() => {
  ui.clearMessages();
  if (!auth.isAuthenticated) {
    void router.replace("/login");
    return;
  }
  void loadLeases();
});
</script>

<template>
  <section class="dashboard-rentals-section card">
    <div class="panel-heading compact dashboard-section-heading">
      <div>
        <h3>我的租赁</h3>
        <p class="muted">包含自己已有的租赁，以及当前租约会话。</p>
      </div>
      <div class="dashboard-form-actions">
        <RouterLink class="btn btn-primary btn-sm" to="/resources">
          <Icon name="cpu-board" :size="14" class="btn-icon" />
          去资源页申请
        </RouterLink>
        <button class="btn btn-ghost btn-sm" :disabled="loading" @click="loadLeases">刷新</button>
      </div>
    </div>

    <section class="dashboard-subsection">
      <div class="dashboard-subsection-head">
        <div>
          <h4>我的预约日历</h4>
          <span>{{ calendarPeriodLabel }}</span>
        </div>
        <div class="lease-calendar-tools">
          <div class="segmented-control lease-calendar-tabs" role="group" aria-label="日历视图">
            <button type="button" :class="{ 'is-active': calendarView === 'hour' }" @click="calendarView = 'hour'">时</button>
            <button type="button" :class="{ 'is-active': calendarView === 'day' }" @click="calendarView = 'day'">日</button>
            <button type="button" :class="{ 'is-active': calendarView === 'month' }" @click="calendarView = 'month'">月</button>
            <button type="button" :class="{ 'is-active': calendarView === 'year' }" @click="calendarView = 'year'">年</button>
          </div>
        </div>
      </div>
      <div v-if="loading" class="empty-state"><div class="empty-state-icon">&#9641;</div>正在加载预约日历...</div>
      <div v-else-if="activeLeases.length === 0" class="empty-state">
        <div class="empty-state-icon">&#9641;</div>
        当前没有活跃租赁，预约日历暂无内容。
      </div>
      <div v-else class="lease-calendar-shell dashboard-lease-calendar-shell">
        <div class="lease-calendar-grid" :class="`lease-calendar-${calendarView}`">
          <article
            v-for="slot in calendarSlots"
            :key="slot.key"
            class="lease-calendar-cell"
            :class="{ 'has-own-reservation': slot.leases.length > 0 }"
          >
            <header>
              <strong>{{ slot.label }}</strong>
              <span v-if="slot.caption">{{ slot.caption }}</span>
            </header>
            <div class="lease-calendar-events">
              <div
                v-for="item in slot.leases.slice(0, calendarView === 'hour' ? 2 : 3)"
                :key="item.lease.id"
                class="lease-calendar-event compact is-own"
                :title="`${formatDateTime(item.lease.starts_at)} ~ ${formatDateTime(item.lease.expires_at)}`"
              >
                <strong>{{ eventLabelForSlot(item, slot) }}</strong>
                <span>{{ item.lease.board_id }}</span>
              </div>
              <span
                v-if="slot.leases.length > (calendarView === 'hour' ? 2 : 3)"
                class="lease-calendar-more"
              >
                +{{ slot.leases.length - (calendarView === 'hour' ? 2 : 3) }} 条
              </span>
            </div>
          </article>
        </div>
      </div>
      <div v-if="!loading && activeLeases.length > 0" class="lease-calendar-nav">
        <button class="btn-icon-only" type="button" title="上一页" @click="moveCalendar(-1)">
          <Icon name="arrow-left" :size="15" />
        </button>
        <button class="btn btn-ghost btn-sm" type="button" @click="focusNow">定位当前</button>
        <button class="btn-icon-only" type="button" title="下一页" @click="moveCalendar(1)">
          <Icon name="arrow-right" :size="15" />
        </button>
      </div>
    </section>

    <section class="dashboard-subsection">
      <div class="dashboard-subsection-head">
        <h4>租赁情况</h4>
        <span>{{ activeLeases.length }} 条活跃</span>
      </div>
      <div v-if="loading" class="empty-state"><div class="empty-state-icon">&#9641;</div>正在加载租赁...</div>
      <div v-else-if="activeLeases.length === 0" class="empty-state">
        <div class="empty-state-icon">&#9641;</div>
        当前没有活跃租赁。请前往资源页面选择开发板并申请租赁。
      </div>

      <div v-else class="board-card-grid">
        <article v-for="item in activeLeases" :key="item.lease.id" class="board-card">
          <div class="board-card-header">
            <div class="board-card-id">
              <strong>{{ item.lease.board_id }}</strong>
              <span class="board-card-meta">{{ item.lease.board_type }} · {{ item.lease.state }}</span>
            </div>
            <span class="pill pill-success">活跃</span>
          </div>

          <dl class="key-value-list lease-card-stats">
            <div><dt>开始时间</dt><dd>{{ formatLeaseTime(item.lease.starts_at) }}</dd></div>
            <div><dt>结束时间</dt><dd>{{ formatLeaseTime(item.lease.expires_at) }}</dd></div>
            <div><dt>剩余时长</dt><dd :style="{color: remainingLeaseLabel(item.lease.expires_at) === '已过期' ? 'var(--c-danger)' : 'var(--c-success)'}">{{ remainingLeaseLabel(item.lease.expires_at) }}</dd></div>
          </dl>

          <div v-if="item.lease.required_tags.length > 0" class="lease-tags">
            <span>标签:</span>
            <span v-for="tag in item.lease.required_tags" :key="tag" class="tag">{{ tag }}</span>
          </div>

          <div class="toolbar-actions">
            <button class="btn btn-danger btn-sm" type="button" @click="releaseLease(item.lease.id)">释放租赁</button>
          </div>
        </article>
      </div>
    </section>

    <section class="dashboard-subsection">
      <div class="dashboard-subsection-head">
        <h4>租约会话</h4>
        <span>{{ activeSessions.length }} 个当前会话</span>
      </div>
      <div v-if="loading" class="empty-state"><div class="empty-state-icon">&#9641;</div>正在加载会话...</div>
      <div v-else-if="activeSessions.length === 0" class="empty-state">
        <div class="empty-state-icon">&#9641;</div>
        当前没有正在运行的租约会话。
      </div>

      <div v-else class="board-card-grid">
        <article v-for="session in activeSessions" :key="session.lease.id" class="board-card">
          <div class="board-card-header">
            <div class="board-card-id">
              <strong>{{ session.lease.board_id }}</strong>
              <span class="board-card-meta">{{ session.lease.board_type }} &middot; {{ session.session?.state ?? session.lease.state }}</span>
            </div>
            <span class="pill pill-success">活跃</span>
          </div>

          <dl class="key-value-list lease-card-stats">
            <div><dt>租赁 ID</dt><dd><code>{{ session.lease.id }}</code></dd></div>
            <div><dt>会话 ID</dt><dd><code>{{ session.lease.session_id || "未启用" }}</code></dd></div>
            <div><dt>到期时间</dt><dd>{{ formatLeaseTime(session.lease.expires_at) }}</dd></div>
            <div><dt>剩余时长</dt><dd :style="{color: remainingLeaseLabel(session.lease.expires_at) === '已过期' ? 'var(--c-danger)' : 'var(--c-success)'}">{{ remainingLeaseLabel(session.lease.expires_at) }}</dd></div>
          </dl>

          <div v-if="session.lease.required_tags.length > 0" class="lease-tags">
            <span>标签:</span>
            <span v-for="tag in session.lease.required_tags" :key="tag" class="tag">{{ tag }}</span>
          </div>

          <div class="toolbar-actions">
            <button class="btn btn-danger btn-sm" type="button" @click="releaseLease(session.lease.id)">释放租赁</button>
          </div>
        </article>
      </div>
    </section>
  </section>
</template>

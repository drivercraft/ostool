<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import { api } from "@/api";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import type { BoardTypeSummary, LeaseResponse } from "@/types/api";

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
const route = useRoute();
const router = useRouter();

const loading = ref(true);
const submitting = ref(false);
const boardTypes = ref<BoardTypeSummary[]>([]);
const leases = ref<LeaseResponse[]>([]);
const selectedBoardType = ref("");
const requiredTags = ref("");
const calendarView = ref<CalendarViewMode>("month");
const calendarCursorIso = ref(new Date().toISOString());

const selectedBoard = computed(() =>
  boardTypes.value.find((board) => board.board_type === selectedBoardType.value) ?? null,
);
const selectedBoardLeases = computed(() =>
  leases.value
    .filter((item) => item.lease.board_type === selectedBoardType.value && item.lease.state === "active")
    .sort((left, right) => new Date(left.lease.starts_at).getTime() - new Date(right.lease.starts_at).getTime()),
);
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
  return selectedBoardLeases.value.filter((item) =>
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
  const end = new Date(anchor);
  end.setHours(0, 0, 0, 0);
  end.setMonth(end.getMonth() + 1);
  const slots: CalendarSlot[] = [];
  for (const slotStart = new Date(start); slotStart <= end; slotStart.setDate(slotStart.getDate() + 1)) {
    const current = new Date(slotStart);
    const slotEnd = new Date(current);
    slotEnd.setDate(current.getDate() + 1);
    slots.push(makeCalendarSlot(formatShortDate(current.toISOString()), "", current, slotEnd));
  }
  return slots;
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
      ? "整点占用"
      : calendarView.value === "day"
        ? "全天占用"
        : calendarView.value === "year"
          ? "整年占用"
          : "整月占用";
  }
  if (start < slotStart) {
    return `截至 ${formatTime(item.lease.expires_at)}`;
  }
  if (end > slotEnd) {
    return `${formatTime(item.lease.starts_at)} 起`;
  }
  return `${formatTime(item.lease.starts_at)} ~ ${formatTime(item.lease.expires_at)}`;
}

function userLabel(item: LeaseResponse) {
  return item.lease.user_id === auth.user?.id ? "我的租赁" : item.lease.user_id;
}

function parseRequiredTags() {
  return requiredTags.value
    .split(",")
    .map((tag) => tag.trim())
    .filter((tag) => tag.length > 0);
}

async function loadData() {
  loading.value = true;
  try {
    const [types, leaseList] = await Promise.all([
      api.listBoardTypes(),
      api.listUserLeases(),
    ]);
    boardTypes.value = types;
    leases.value = leaseList.leases;
    if (!selectedBoardType.value && types.length > 0) {
      const candidate = types.find((item) => item.available > 0);
      selectedBoardType.value = candidate?.board_type ?? types[0].board_type;
    }
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

async function createLease() {
  if (!selectedBoardType.value) {
    ui.setError("请选择开发板型号");
    return;
  }
  submitting.value = true;
  try {
    const created = await api.createLease({
      board_type: selectedBoardType.value,
      required_tags: parseRequiredTags(),
    });
    ui.setSuccess(`已创建租赁 ${created.lease.id}，开发板 ${created.lease.board_id} 已分配`);
    await router.push("/dashboard#leases");
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    submitting.value = false;
  }
}

watch(
  () => route.query.board_type,
  (boardType) => {
    selectedBoardType.value = typeof boardType === "string" ? boardType : "";
  },
  { immediate: true },
);

onMounted(() => {
  ui.clearMessages();
  void loadData();
});
</script>

<template>
  <section class="page-grid admin-editor-page user-lease-create-page">
    <div class="admin-editor-panel panel">
      <div class="admin-editor-header">
        <div>
          <h3>申请租赁</h3>
          <p class="muted">选择开发板型号后查看我的相关占用情况，再创建即时租赁会话。</p>
        </div>
        <RouterLink class="btn btn-ghost btn-sm" to="/resources">返回资源</RouterLink>
      </div>

      <form class="admin-editor-form" @submit.prevent="createLease">
        <div class="admin-editor-body lease-editor-scroll">
          <section class="lease-calendar-panel">
            <div class="lease-calendar-head">
              <div>
                <h4>{{ selectedBoardType || "请选择开发板型号" }}</h4>
                <p class="muted">{{ calendarPeriodLabel }} 的我的租赁占用情况</p>
              </div>
              <div class="lease-calendar-tools">
                <div class="segmented-control lease-calendar-tabs" role="group" aria-label="日历视图">
                  <button type="button" :class="{ 'is-active': calendarView === 'hour' }" @click="calendarView = 'hour'">时</button>
                  <button type="button" :class="{ 'is-active': calendarView === 'day' }" @click="calendarView = 'day'">日</button>
                  <button type="button" :class="{ 'is-active': calendarView === 'month' }" @click="calendarView = 'month'">月</button>
                  <button type="button" :class="{ 'is-active': calendarView === 'year' }" @click="calendarView = 'year'">年</button>
                </div>
                <div class="lease-calendar-nav">
                  <button class="btn-icon-only" type="button" title="上一页" @click="moveCalendar(-1)">
                    <Icon name="arrow-left" :size="15" />
                  </button>
                  <button class="btn btn-ghost btn-sm" type="button" @click="focusNow">定位当前</button>
                  <button class="btn-icon-only" type="button" title="下一页" @click="moveCalendar(1)">
                    <Icon name="arrow-right" :size="15" />
                  </button>
                </div>
              </div>
            </div>

            <div v-if="loading" class="empty-state">正在加载租赁日历...</div>
            <div v-else-if="!selectedBoard" class="empty-state">请选择可租赁的开发板型号。</div>
            <div v-else class="lease-calendar-shell">
              <div class="lease-calendar-grid" :class="`lease-calendar-${calendarView}`">
                <article
                  v-for="slot in calendarSlots"
                  :key="slot.key"
                  class="lease-calendar-cell"
                  :class="{ 'has-reservation': slot.leases.length > 0 }"
                >
                  <header>
                    <strong>{{ slot.label }}</strong>
                    <span v-if="slot.caption">{{ slot.caption }}</span>
                  </header>
                  <div class="lease-calendar-events">
                    <div
                      v-for="item in slot.leases.slice(0, calendarView === 'hour' ? 2 : 3)"
                      :key="item.lease.id"
                      class="lease-calendar-event compact"
                      :title="`${formatDateTime(item.lease.starts_at)} ~ ${formatDateTime(item.lease.expires_at)}`"
                    >
                      <strong>{{ eventLabelForSlot(item, slot) }}</strong>
                      <span>{{ userLabel(item) }}</span>
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
          </section>

          <section class="lease-config-panel">
            <div class="form-section">
              <div class="form-section-header">
                <span class="form-section-icon info"><Icon name="clipboard" :size="16" /></span>
                <h4>租赁配置</h4>
              </div>

              <div class="form-grid">
                <label class="field is-required">
                  <span>开发板型号</span>
                  <select v-model="selectedBoardType" :disabled="loading || submitting">
                    <option value="" disabled>请选择开发板型号</option>
                    <option v-for="board in boardTypes" :key="board.board_type" :value="board.board_type">
                      {{ board.board_type }} / 可用 {{ board.available }} / {{ board.total }}
                    </option>
                  </select>
                </label>

                <label class="field">
                  <span>必需标签（选填）</span>
                  <input
                    v-model="requiredTags"
                    maxlength="256"
                    :disabled="submitting"
                    placeholder="多个标签用英文逗号分隔，例如 lab, usb"
                  />
                </label>
              </div>
            </div>

            <div class="form-section">
              <div class="form-section-header">
                <span class="form-section-icon boot"><Icon name="pulse" :size="16" /></span>
                <h4>租赁说明</h4>
              </div>

              <dl class="lease-summary-list">
                <div>
                  <dt>创建方式</dt>
                  <dd>提交后立即分配当前空闲开发板并创建会话。</dd>
                </div>
                <div>
                  <dt>时间段</dt>
                  <dd>普通用户租期由系统默认租赁策略决定。</dd>
                </div>
                <div>
                  <dt>当前型号</dt>
                  <dd>{{ selectedBoard ? `${selectedBoard.available} / ${selectedBoard.total} 可用` : "-" }}</dd>
                </div>
              </dl>
            </div>
          </section>
        </div>

        <div class="admin-editor-actions">
          <button type="submit" class="btn btn-primary" :disabled="submitting || loading || !selectedBoardType">
            {{ submitting ? "申请中..." : "创建租赁" }}
          </button>
          <RouterLink class="btn btn-ghost" to="/resources">取消</RouterLink>
        </div>
      </form>
    </div>
  </section>
</template>

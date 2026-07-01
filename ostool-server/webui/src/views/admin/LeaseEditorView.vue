<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";

import Icon from "@/components/Icon.vue";
import { api } from "@/api";
import { VALIDATION_LIMITS } from "@/constants/validation";
import { useUiStore } from "@/stores/ui";
import type { AdminUserResponse, BoardConfig, LeaseResponse } from "@/types/api";

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
const route = useRoute();
const router = useRouter();

const loading = ref(true);
const saving = ref(false);
const leases = ref<LeaseResponse[]>([]);
const users = ref<AdminUserResponse[]>([]);
const boards = ref<BoardConfig[]>([]);
const calendarView = ref<CalendarViewMode>("month");
const calendarCursorIso = ref(new Date().toISOString());
const editingLease = ref<LeaseResponse | null>(null);
const form = ref({
  user_id: "",
  board_id: "",
  client_name: "",
  starts_at: "",
  expires_at: "",
  failure_message: "",
});

const editing = computed(() => route.name === "admin-rental-lease-edit");
const editingLeaseId = computed(() => (typeof route.params.leaseId === "string" ? route.params.leaseId : ""));
const enabledUsers = computed(() => users.value.filter((user) => !user.disabled));
const enabledBoards = computed(() => boards.value.filter((board) => !board.disabled));
const selectedBoard = computed(() => boards.value.find((board) => board.id === form.value.board_id) ?? null);
const selectedDate = computed(() => form.value.starts_at.slice(0, 10));
const selectedBoardLeases = computed(() =>
  leases.value
    .filter((item) => item.lease.board_id === form.value.board_id && item.lease.state === "active")
    .sort((left, right) => new Date(left.lease.starts_at).getTime() - new Date(right.lease.starts_at).getTime()),
);
const selectedWindowConflicts = computed(() => {
  if (!form.value.starts_at || !form.value.expires_at) {
    return [];
  }
  return selectedBoardLeases.value.filter((item) =>
    item.lease.id !== editingLeaseId.value
    &&
    windowsOverlap(
      item.lease.starts_at,
      item.lease.expires_at,
      fromDatetimeLocal(form.value.starts_at),
      fromDatetimeLocal(form.value.expires_at),
    ),
  );
});
const hasConflict = computed(() => selectedWindowConflicts.value.length > 0);
const calendarAnchor = computed(() => {
  const date = new Date(calendarCursorIso.value);
  return Number.isNaN(date.getTime()) ? new Date() : date;
});
const calendarDate = computed(() => dateKey(calendarAnchor.value));
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
  const base = calendarAnchor.value;
  if (calendarView.value === "hour") {
    const first = calendarSlots.value[0];
    const last = calendarSlots.value[calendarSlots.value.length - 1];
    return `${formatDateTime(first.startIso)} ~ ${formatDateTime(last.endIso)}`;
  }
  if (calendarView.value === "day") {
    const first = calendarSlots.value[0];
    const last = calendarSlots.value[calendarSlots.value.length - 1];
    return `${formatDate(first.startIso)} ~ ${formatDate(last.startIso)}`;
  }
  if (calendarView.value === "month") {
    const first = calendarSlots.value[0];
    const last = calendarSlots.value[calendarSlots.value.length - 1];
    return `${formatMonth(first.startIso)} ~ ${formatMonth(last.startIso)}`;
  }
  if (calendarView.value === "year") {
    const first = calendarSlots.value[0];
    const last = calendarSlots.value[calendarSlots.value.length - 1];
    return `${new Date(first.startIso).getFullYear()} ~ ${new Date(last.startIso).getFullYear()}`;
  }
  return base.toLocaleDateString("zh-CN", { year: "numeric", month: "long" });
});

function toDatetimeLocal(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "";
  }
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60000);
  return local.toISOString().slice(0, 16);
}

function fromDatetimeLocal(value: string) {
  return new Date(value).toISOString();
}

function dateKey(date: Date) {
  const year = date.getFullYear();
  const month = `${date.getMonth() + 1}`.padStart(2, "0");
  const day = `${date.getDate()}`.padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function defaultStartsAt() {
  return toDatetimeLocal(new Date().toISOString());
}

function defaultExpiresAt(startsAt: string) {
  const startTime = startsAt ? new Date(startsAt).getTime() : Date.now();
  return toDatetimeLocal(new Date(startTime + 60 * 60 * 1000).toISOString());
}

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

function focusReservationDate() {
  if (form.value.starts_at) {
    calendarCursorIso.value = fromDatetimeLocal(form.value.starts_at);
  }
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
  start.setDate(start.getDate() - 17);
  return Array.from({ length: 35 }, (_, index) => {
    const slotStart = new Date(start);
    slotStart.setDate(start.getDate() + index);
    const current = new Date(slotStart);
    const slotEnd = new Date(current);
    slotEnd.setDate(current.getDate() + 1);
    return makeCalendarSlot(
      formatShortDate(current.toISOString()),
      "",
      current,
      slotEnd,
    );
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
    const slotStart = new Date(year, 0, 1);
    const slotEnd = new Date(year + 1, 0, 1);
    return makeCalendarSlot(`${year}`, "全年", slotStart, slotEnd);
  });
}

function slotOverlapsSelected(slot: CalendarSlot) {
  if (!form.value.starts_at || !form.value.expires_at) {
    return false;
  }
  return windowsOverlap(
    slot.startIso,
    slot.endIso,
    fromDatetimeLocal(form.value.starts_at),
    fromDatetimeLocal(form.value.expires_at),
  );
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

function durationLabel(start: string, end: string) {
  const startTime = new Date(start).getTime();
  const endTime = new Date(end).getTime();
  if (!Number.isFinite(startTime) || !Number.isFinite(endTime) || endTime <= startTime) {
    return "-";
  }
  const minutes = Math.round((endTime - startTime) / 60000);
  if (minutes < 60) {
    return `${minutes} 分钟`;
  }
  const hours = Math.floor(minutes / 60);
  const remain = minutes % 60;
  return remain ? `${hours} 小时 ${remain} 分钟` : `${hours} 小时`;
}

function selectedDurationLabel() {
  if (!form.value.starts_at || !form.value.expires_at) {
    return "-";
  }
  return durationLabel(form.value.starts_at, form.value.expires_at);
}

function userLabel(userId: string) {
  const user = users.value.find((item) => item.id === userId);
  return user ? `${user.display_name || user.username} / ${user.username}` : userId;
}

function isConflictLease(item: LeaseResponse) {
  return selectedWindowConflicts.value.some((conflict) => conflict.lease.id === item.lease.id);
}

function fillFormFromLease(item: LeaseResponse) {
  editingLease.value = item;
  form.value = {
    user_id: item.lease.user_id,
    board_id: item.lease.board_id,
    client_name: item.session?.client_name ?? "",
    starts_at: toDatetimeLocal(item.lease.starts_at),
    expires_at: toDatetimeLocal(item.lease.expires_at),
    failure_message: item.lease.failure_message ?? "",
  };
  calendarCursorIso.value = item.lease.starts_at;
}

async function loadData() {
  loading.value = true;
  try {
    const [leaseResponse, userResponse, boardResponse] = await Promise.all([
      api.admin.listAdminLeases(),
      api.admin.listAdminUsers(),
      api.admin.listBoards(),
    ]);
    leases.value = leaseResponse.leases;
    users.value = userResponse.users;
    boards.value = boardResponse;
    if (editing.value) {
      const current = leases.value.find((item) => item.lease.id === editingLeaseId.value);
      if (!current) {
        ui.setError("未找到要编辑的租赁");
        await router.replace({ name: "admin-rental-leases" });
        return;
      }
      fillFormFromLease(current);
    } else {
      form.value.user_id = enabledUsers.value[0]?.id ?? "";
      form.value.board_id = enabledBoards.value[0]?.id ?? "";
    }
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

function validateForm() {
  if (!form.value.user_id) {
    return "请选择用户";
  }
  if (!form.value.board_id) {
    return "请选择开发板";
  }
  if (!form.value.starts_at || !form.value.expires_at) {
    return "请填写租赁开始和结束时间";
  }
  if (new Date(form.value.expires_at).getTime() <= new Date(form.value.starts_at).getTime()) {
    return "租赁结束时间必须晚于开始时间";
  }
  if (!editing.value && form.value.client_name.trim().length > VALIDATION_LIMITS.clientNameMax) {
    return `会话名称不能超过 ${VALIDATION_LIMITS.clientNameMax} 个字符`;
  }
  if (editing.value && form.value.failure_message.trim().length > VALIDATION_LIMITS.longDescriptionMax) {
    return `备注 / 失败信息不能超过 ${VALIDATION_LIMITS.longDescriptionMax} 个字符`;
  }
  if (hasConflict.value) {
    return "当前时间段与已有租赁冲突，请调整预约时间";
  }
  return "";
}

async function saveLease() {
  const error = validateForm();
  if (error) {
    ui.setError(error);
    return;
  }
  saving.value = true;
  try {
    if (editing.value) {
      await api.admin.updateAdminLease(editingLeaseId.value, {
        starts_at: fromDatetimeLocal(form.value.starts_at),
        expires_at: fromDatetimeLocal(form.value.expires_at),
        failure_message: form.value.failure_message.trim() || null,
      });
      ui.setSuccess("租赁已更新");
    } else {
      await api.admin.createAdminLease({
        user_id: form.value.user_id,
        board_id: form.value.board_id,
        client_name: form.value.client_name.trim() || null,
        starts_at: fromDatetimeLocal(form.value.starts_at),
        expires_at: fromDatetimeLocal(form.value.expires_at),
      });
      ui.setSuccess("租赁已创建");
    }
    await router.push({ name: "admin-rental-leases" });
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    saving.value = false;
  }
}

watch(
  () => form.value.starts_at,
  (startsAt, oldStartsAt) => {
    if (startsAt) {
      calendarCursorIso.value = fromDatetimeLocal(startsAt);
    }
    if (!startsAt || form.value.expires_at) {
      return;
    }
    form.value.expires_at = defaultExpiresAt(startsAt || oldStartsAt || "");
  },
);

onMounted(() => {
  const startsAt = defaultStartsAt();
  form.value.starts_at = startsAt;
  form.value.expires_at = defaultExpiresAt(startsAt);
  form.value.failure_message = "";
  void loadData();
});
</script>

<template>
  <section class="page-grid admin-editor-page lease-editor-page">
    <div class="admin-editor-panel panel">
      <form class="admin-editor-form" @submit.prevent="saveLease">
        <div class="admin-editor-body lease-editor-scroll">
          <section class="lease-calendar-panel">
            <div class="lease-calendar-head">
              <div>
                <h4>{{ selectedBoard?.id || "请选择开发板" }}</h4>
                <p class="muted">{{ calendarPeriodLabel }} 的预约占用情况</p>
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

            <div v-if="loading" class="empty-state">正在加载租赁日历...</div>
            <div v-else-if="!selectedBoard" class="empty-state">暂无开发板数据</div>
            <div v-else class="lease-calendar-shell">
              <div
                class="lease-calendar-grid"
                :class="`lease-calendar-${calendarView}`"
              >
                <article
                  v-for="slot in calendarSlots"
                  :key="slot.key"
                  class="lease-calendar-cell"
                  :class="{
                    'is-selected': slotOverlapsSelected(slot),
                    'has-reservation': slot.leases.length > 0,
                  }"
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
                      :class="{ 'is-conflicting': isConflictLease(item) }"
                      :title="`${formatDateTime(item.lease.starts_at)} ~ ${formatDateTime(item.lease.expires_at)}`"
                    >
                      <strong>{{ eventLabelForSlot(item, slot) }}</strong>
                      <span>{{ userLabel(item.lease.user_id) }}</span>
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
            <div v-if="!loading && selectedBoard" class="lease-calendar-nav">
              <button class="btn-icon-only" type="button" title="上一页" @click="moveCalendar(-1)">
                <Icon name="arrow-left" :size="15" />
              </button>
              <button class="btn btn-ghost btn-sm" type="button" @click="focusReservationDate">定位预约</button>
              <button class="btn-icon-only" type="button" title="下一页" @click="moveCalendar(1)">
                <Icon name="arrow-right" :size="15" />
              </button>
            </div>
          </section>

          <section class="lease-config-panel">
            <div class="form-section">
              <div class="form-section-header">
                <span class="form-section-icon info"><Icon name="clipboard" :size="16" /></span>
                <h4>租赁配置</h4>
              </div>

              <div class="form-grid">
                <label class="field select-field is-required">
                  <span>开发板</span>
                  <select v-model="form.board_id" :disabled="loading || editing">
                    <option value="" disabled>请选择开发板</option>
                    <option v-for="board in enabledBoards" :key="board.id" :value="board.id">
                      {{ board.id }} / {{ board.board_type }}
                    </option>
                  </select>
                </label>

                <label class="field select-field is-required">
                  <span>用户</span>
                  <select v-model="form.user_id" :disabled="loading || editing">
                    <option value="" disabled>请选择用户</option>
                    <option v-for="user in enabledUsers" :key="user.id" :value="user.id">
                      {{ user.display_name || user.username }} / {{ user.username }}
                    </option>
                  </select>
                </label>

                <label class="field">
                  <span>会话名称（选填）</span>
                  <input
                    v-model="form.client_name"
                    :maxlength="VALIDATION_LIMITS.clientNameMax"
                    :disabled="editing"
                    placeholder="例如 手动分配给 Alice"
                  />
                </label>

                <label v-if="editing" class="field">
                  <span>备注 / 失败信息</span>
                  <input
                    v-model="form.failure_message"
                    :maxlength="VALIDATION_LIMITS.longDescriptionMax"
                    placeholder="可选"
                  />
                </label>
              </div>
            </div>

            <div class="form-section">
              <div class="form-section-header">
                <span class="form-section-icon boot"><Icon name="pulse" :size="16" /></span>
                <h4>预约时间</h4>
              </div>

              <div class="form-grid two-columns">
                <label class="field is-required">
                  <span>开始时间</span>
                  <input v-model="form.starts_at" type="datetime-local" />
                </label>
                <label class="field is-required">
                  <span>结束时间</span>
                  <input v-model="form.expires_at" type="datetime-local" />
                </label>
              </div>

              <p v-if="hasConflict" class="field-error">
                当前时间段与 {{ selectedWindowConflicts.length }} 条已有租赁冲突。
              </p>
              <p v-else class="field-hint">
                当前时间段可预约，预计使用 {{ selectedDurationLabel() }}。
              </p>
            </div>
          </section>
        </div>

        <div class="admin-editor-actions">
          <button type="submit" class="btn btn-primary" :disabled="saving || loading">
            {{ saving ? "保存中..." : "保存" }}
          </button>
          <RouterLink class="btn btn-ghost" to="/admin/rentals/leases">取消</RouterLink>
        </div>
      </form>
    </div>
  </section>
</template>

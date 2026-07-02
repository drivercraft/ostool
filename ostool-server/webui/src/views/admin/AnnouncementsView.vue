<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";

import { api } from "@/api";
import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import type {
  Announcement,
  AnnouncementKind,
  AnnouncementResponse,
  AnnouncementStatus,
} from "@/types/api";
import {
  getAnnouncementKindLabel,
  getAnnouncementStatusDisplay,
} from "@/utils/announcement";

const ui = useUiStore();
const auth = useAuthStore();

const loading = ref(true);
const submitting = ref(false);
const announcements = ref<AnnouncementResponse[]>([]);
const search = ref("");
const statusFilter = ref<AnnouncementStatus | "all">("all");
const kindFilter = ref<AnnouncementKind | "all">("all");
const editingAnnouncement = ref<Announcement | null>(null);
const modalVisible = ref(false);
const modalMode = ref<"create" | "edit">("create");
const form = reactive<{
  title: string;
  content: string;
  kind: AnnouncementKind;
  status: AnnouncementStatus;
  pinned: boolean;
}>({
  title: "",
  content: "",
  kind: "system",
  status: "draft",
  pinned: false,
});

const canCreate = computed(() => auth.hasPermission("announcements.create"));
const canUpdate = computed(() => auth.hasPermission("announcements.update"));
const canDelete = computed(() => auth.hasPermission("announcements.delete"));

const filteredAnnouncements = computed(() =>
  announcements.value.filter(({ announcement }) => {
    if (statusFilter.value !== "all" && announcement.status !== statusFilter.value) {
      return false;
    }
    if (kindFilter.value !== "all" && announcement.kind !== kindFilter.value) {
      return false;
    }
    const query = search.value.trim().toLowerCase();
    if (!query) {
      return true;
    }
    return [
      announcement.id,
      announcement.title,
      announcement.content,
      getAnnouncementKindLabel(announcement.kind),
    ].some((value) => value.toLowerCase().includes(query));
  }),
);

const formReady = computed(() =>
  form.title.trim().length > 0
    && form.title.trim().length <= 128
    && form.content.trim().length > 0
    && form.content.trim().length <= 8000,
);

function resetForm() {
  form.title = "";
  form.content = "";
  form.kind = "system";
  form.status = "draft";
  form.pinned = false;
}

function openCreate() {
  modalMode.value = "create";
  editingAnnouncement.value = null;
  resetForm();
  modalVisible.value = true;
}

function openEdit(announcement: Announcement) {
  modalMode.value = "edit";
  editingAnnouncement.value = announcement;
  form.title = announcement.title;
  form.content = announcement.content;
  form.kind = announcement.kind;
  form.status = announcement.status;
  form.pinned = announcement.pinned;
  modalVisible.value = true;
}

function closeModal() {
  if (submitting.value) {
    return;
  }
  editingAnnouncement.value = null;
  modalVisible.value = false;
}

function formatDateTime(value: string | null) {
  return value ? new Date(value).toLocaleString("zh-CN", { hour12: false }) : "-";
}

async function loadAnnouncements() {
  loading.value = true;
  try {
    const response = await api.admin.listAnnouncements();
    announcements.value = response.announcements;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

async function submitAnnouncement() {
  if (!formReady.value) {
    ui.setError("请填写公告标题和公告内容");
    return;
  }
  if (modalMode.value === "create" && !canCreate.value) {
    ui.setError("缺少新增公告权限");
    return;
  }
  if (modalMode.value === "edit" && !canUpdate.value) {
    ui.setError("缺少编辑公告权限");
    return;
  }
  submitting.value = true;
  try {
    const payload = {
      title: form.title.trim(),
      content: form.content.trim(),
      kind: form.kind,
      status: form.status,
      pinned: form.pinned,
    };
    const saved = modalMode.value === "create"
      ? await api.admin.createAnnouncement(payload)
      : await api.admin.updateAnnouncement(editingAnnouncement.value!.id, payload);
    announcements.value = [
      saved,
      ...announcements.value.filter((item) => item.announcement.id !== saved.announcement.id),
    ].sort((a, b) => {
      if (a.announcement.pinned !== b.announcement.pinned) {
        return a.announcement.pinned ? -1 : 1;
      }
      return b.announcement.created_at.localeCompare(a.announcement.created_at);
    });
    ui.setSuccess(modalMode.value === "create" ? "公告已创建" : "公告已更新");
    editingAnnouncement.value = null;
    modalVisible.value = false;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    submitting.value = false;
  }
}

async function deleteAnnouncement(announcement: Announcement) {
  if (!canDelete.value) {
    ui.setError("缺少删除公告权限");
    return;
  }
  const confirmed = await ui.confirm({
    tone: "danger",
    title: "删除公告",
    message: `确认删除公告“${announcement.title}”？`,
    confirmLabel: "删除",
  });
  if (!confirmed) {
    return;
  }
  try {
    await api.admin.deleteAnnouncement(announcement.id);
    announcements.value = announcements.value.filter((item) => item.announcement.id !== announcement.id);
    ui.setSuccess("公告已删除");
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

onMounted(() => {
  ui.clearMessages();
  void loadAnnouncements();
});
</script>

<template>
  <section class="page-grid admin-list-page admin-list-content">
      <div class="admin-toolbar">
        <div class="admin-toolbar-left">
          <button class="btn btn-primary" type="button" :disabled="!canCreate" @click="openCreate">
            <Icon name="plus" :size="16" class="btn-icon" />
            新增公告
          </button>
        </div>
        <div class="admin-toolbar-right">
          <label class="search-field">
            <Icon name="search" :size="16" />
            <input v-model="search" type="search" maxlength="128" placeholder="搜索公告标题 / 内容" />
          </label>
          <label class="field select-field filter-field">
            <span>类型</span>
            <select v-model="kindFilter">
              <option value="all">全部类型</option>
              <option value="system">系统公告</option>
              <option value="activity">活动公告</option>
            </select>
          </label>
          <label class="field select-field filter-field">
            <span>状态</span>
            <select v-model="statusFilter">
              <option value="all">全部状态</option>
              <option value="draft">草稿</option>
              <option value="published">已发布</option>
              <option value="hidden">已隐藏</option>
            </select>
          </label>
        </div>
      </div>

      <div v-if="loading" class="empty-state">正在加载公告...</div>
      <div v-else class="table-scroll">
        <table class="data-table">
          <thead>
            <tr>
              <th class="col-index">序号</th>
              <th>公告</th>
              <th>类型</th>
              <th>状态</th>
              <th>置顶</th>
              <th>发布时间</th>
              <th>更新时间</th>
              <th class="col-actions">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(item, index) in filteredAnnouncements" :key="item.announcement.id">
              <td class="col-index">{{ index + 1 }}</td>
              <td>
                <div class="table-cell-stack">
                  <div class="table-cell-stack-body">
                    <span class="table-cell-main">{{ item.announcement.title }}</span>
                    <span class="table-cell-sub">{{ item.announcement.content }}</span>
                  </div>
                </div>
              </td>
              <td>{{ getAnnouncementKindLabel(item.announcement.kind) }}</td>
              <td>
                <StatusPill
                  :tone="getAnnouncementStatusDisplay(item.announcement.status).tone"
                  :label="getAnnouncementStatusDisplay(item.announcement.status).label"
                />
              </td>
              <td>{{ item.announcement.pinned ? "是" : "否" }}</td>
              <td>{{ formatDateTime(item.announcement.published_at) }}</td>
              <td>{{ formatDateTime(item.announcement.updated_at) }}</td>
              <td class="col-actions">
                <div class="row-actions">
                  <button
                    class="btn-icon-only"
                    type="button"
                    title="编辑公告"
                    :disabled="!canUpdate"
                    @click="openEdit(item.announcement)"
                  >
                    <Icon name="edit" :size="16" />
                  </button>
                  <button
                    class="btn-icon-only"
                    type="button"
                    title="删除公告"
                    :disabled="!canDelete"
                    @click="deleteAnnouncement(item.announcement)"
                  >
                    <Icon name="trash" :size="16" />
                  </button>
                </div>
              </td>
            </tr>
            <tr v-if="filteredAnnouncements.length === 0" class="table-empty-row">
              <td colspan="8">
                <div class="empty-state">暂无公告数据</div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div v-if="!loading" class="table-statusbar">
        <span>{{ filteredAnnouncements.length === 0 ? "暂无分页" : "第 1 / 共 1 页" }}</span>
        <span>本页 {{ filteredAnnouncements.length }} 条</span>
        <span>筛选后 {{ filteredAnnouncements.length }} 条 / 共 {{ announcements.length }} 条</span>
      </div>
    <div v-if="modalVisible" class="modal-overlay">
      <div class="modal-card">
        <header class="modal-header">
          <h3>{{ modalMode === "create" ? "新增公告" : "编辑公告" }}</h3>
          <button class="btn-icon-only modal-close-button" type="button" title="关闭" @click="closeModal">×</button>
        </header>

        <form class="modal-form" @submit.prevent="submitAnnouncement">
          <div class="modal-body modal-body-grid">
            <label class="field select-field">
              <span>公告类型</span>
              <select v-model="form.kind">
                <option value="system">系统公告</option>
                <option value="activity">活动公告</option>
              </select>
            </label>
            <label class="field select-field">
              <span>公告状态</span>
              <select v-model="form.status">
                <option value="draft">草稿</option>
                <option value="published">已发布</option>
                <option value="hidden">已隐藏</option>
              </select>
            </label>
            <label class="field is-required modal-field-full">
              <span>公告标题</span>
              <input v-model="form.title" type="text" maxlength="128" placeholder="请输入公告标题" />
            </label>
            <label class="checkbox-field modal-field-full">
              <input v-model="form.pinned" type="checkbox" />
              <span>置顶显示</span>
            </label>
            <label class="field is-required modal-field-full">
              <span>公告内容</span>
              <textarea
                v-model="form.content"
                maxlength="8000"
                rows="8"
                placeholder="请输入公告内容"
              />
            </label>
          </div>

          <div class="modal-actions toolbar-actions">
            <button type="submit" class="btn btn-primary" :disabled="!formReady || submitting">
              {{ submitting ? "保存中..." : "保存" }}
            </button>
            <button type="button" class="btn btn-ghost" :disabled="submitting" @click="closeModal">取消</button>
          </div>
        </form>
      </div>
    </div>
  </section>
</template>

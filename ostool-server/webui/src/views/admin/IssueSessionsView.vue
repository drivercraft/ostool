<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";

import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import type {
  AdminUserResponse,
  IssueSession,
  IssueSessionPriority,
  IssueSessionResponse,
  IssueSessionState,
} from "@/types/api";
import {
  getIssueCategoryLabel,
  getIssuePriorityLabel,
  getIssueStateDisplay,
} from "@/utils/issueSession";

const ui = useUiStore();
const auth = useAuthStore();

const loading = ref(true);
const submitting = ref(false);
const issues = ref<IssueSessionResponse[]>([]);
const users = ref<AdminUserResponse[]>([]);
const search = ref("");
const stateFilter = ref<IssueSessionState | "all">("all");
const priorityFilter = ref<IssueSessionPriority | "all">("all");
const editingIssue = ref<IssueSession | null>(null);
const editForm = reactive<{
  state: IssueSessionState;
  priority: IssueSessionPriority;
  resolution: string;
}>({
  state: "open",
  priority: "normal",
  resolution: "",
});

const canUpdateIssues = computed(() => auth.hasPermission("issues.update"));
const canDeleteIssues = computed(() => auth.hasPermission("issues.delete"));
const userMap = computed(() => new Map(users.value.map((user) => [user.id, user])));

const filteredIssues = computed(() =>
  issues.value.filter(({ issue }) => {
    if (stateFilter.value !== "all" && issue.state !== stateFilter.value) {
      return false;
    }
    if (priorityFilter.value !== "all" && issue.priority !== priorityFilter.value) {
      return false;
    }
    const query = search.value.trim().toLowerCase();
    if (!query) {
      return true;
    }
    return [
      issue.id,
      issue.title,
      issue.category,
      issue.description,
      issue.lease_id ?? "",
      issue.session_id ?? "",
      issue.user_id,
      userLabel(issue.user_id),
      issue.resolution ?? "",
    ].some((value) => value.toLowerCase().includes(query));
  }),
);

function userLabel(userId: string) {
  const user = userMap.value.get(userId);
  return user ? user.display_name || user.username : userId;
}

function openEdit(issue: IssueSession) {
  editingIssue.value = issue;
  editForm.state = issue.state;
  editForm.priority = issue.priority;
  editForm.resolution = issue.resolution ?? "";
}

function closeEdit() {
  if (submitting.value) {
    return;
  }
  editingIssue.value = null;
}

async function loadIssues() {
  loading.value = true;
  try {
    const [issueList, userList] = await Promise.all([
      api.admin.listIssueSessions(),
      api.admin.listAdminUsers(),
    ]);
    issues.value = issueList.issues;
    users.value = userList.users;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

async function submitEdit() {
  if (!editingIssue.value || !canUpdateIssues.value) {
    ui.setError("缺少处理问题会话权限");
    return;
  }
  submitting.value = true;
  try {
    const updated = await api.admin.updateIssueSession(editingIssue.value.id, {
      state: editForm.state,
      priority: editForm.priority,
      resolution: editForm.resolution.trim() || null,
    });
    issues.value = issues.value.map((item) =>
      item.issue.id === updated.issue.id ? updated : item,
    );
    ui.setSuccess(`已更新问题会话 ${updated.issue.id}`);
    editingIssue.value = null;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    submitting.value = false;
  }
}

async function deleteIssue(issue: IssueSession) {
  if (!canDeleteIssues.value) {
    ui.setError("缺少删除问题会话权限");
    return;
  }
  const confirmed = await ui.confirm({
    tone: "danger",
    title: "删除问题会话",
    message: `确认删除问题会话 ${issue.id} 吗？`,
    confirmLabel: "删除",
  });
  if (!confirmed) {
    return;
  }
  try {
    await api.admin.deleteIssueSession(issue.id);
    ui.setSuccess(`已删除问题会话 ${issue.id}`);
    await loadIssues();
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

onMounted(() => {
  ui.clearMessages();
  void loadIssues();
});
</script>

<template>
  <section class="page-grid admin-list-page admin-list-content">
      <div class="admin-toolbar">
        <div class="admin-toolbar-left"></div>
        <div class="admin-toolbar-right">
          <label class="search-field">
            <Icon name="search" :size="16" />
            <input v-model="search" type="search" maxlength="128" placeholder="搜索问题 / 用户 / 租赁 / 会话" />
          </label>
          <label class="field select-field filter-field">
            <span>状态</span>
            <select v-model="stateFilter">
              <option value="all">全部状态</option>
              <option value="open">待处理</option>
              <option value="in_progress">处理中</option>
              <option value="resolved">已解决</option>
              <option value="closed">已关闭</option>
            </select>
          </label>
          <label class="field select-field filter-field">
            <span>优先级</span>
            <select v-model="priorityFilter">
              <option value="all">全部优先级</option>
              <option value="low">低</option>
              <option value="normal">普通</option>
              <option value="high">高</option>
              <option value="urgent">紧急</option>
            </select>
          </label>
        </div>
      </div>

      <div v-if="loading" class="empty-state">正在加载问题会话...</div>
      <div v-else class="table-scroll">
        <table class="data-table">
          <thead>
            <tr>
              <th class="col-index">序号</th>
              <th>问题</th>
              <th>反馈用户</th>
              <th>关联资源</th>
              <th>分类</th>
              <th>优先级</th>
              <th>状态</th>
              <th>反馈时间</th>
              <th class="col-actions">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(item, index) in filteredIssues" :key="item.issue.id">
              <td class="col-index">{{ index + 1 }}</td>
              <td>
                <div class="table-cell-stack">
                  <div class="table-cell-stack-body">
                    <span class="table-cell-main">{{ item.issue.title }}</span>
                    <span class="table-cell-sub">{{ item.issue.description }}</span>
                  </div>
                </div>
              </td>
              <td>
                <div class="table-cell-stack">
                  <div class="table-cell-stack-body">
                    <span class="table-cell-main">{{ userLabel(item.issue.user_id) }}</span>
                    <span class="table-cell-sub">{{ item.issue.user_id }}</span>
                  </div>
                </div>
              </td>
              <td>
                <div class="table-cell-stack">
                  <div class="table-cell-stack-body">
                    <span class="table-cell-main">租赁 {{ item.issue.lease_id || "-" }}</span>
                    <span class="table-cell-sub">会话 {{ item.issue.session_id || "-" }}</span>
                  </div>
                </div>
              </td>
              <td>{{ getIssueCategoryLabel(item.issue.category) }}</td>
              <td>{{ getIssuePriorityLabel(item.issue.priority) }}</td>
              <td>
                <StatusPill
                  :tone="getIssueStateDisplay(item.issue.state).tone"
                  :label="getIssueStateDisplay(item.issue.state).label"
                />
              </td>
              <td>{{ new Date(item.issue.created_at).toLocaleString() }}</td>
              <td class="col-actions">
                <div class="row-actions">
                  <button
                    class="btn-icon-only"
                    title="处理"
                    :disabled="!canUpdateIssues"
                    @click="openEdit(item.issue)"
                  >
                    <Icon name="edit" :size="16" />
                  </button>
                  <button
                    class="btn-icon-only"
                    title="删除"
                    :disabled="!canDeleteIssues"
                    @click="deleteIssue(item.issue)"
                  >
                    <Icon name="trash" :size="16" />
                  </button>
                </div>
              </td>
            </tr>
            <tr v-if="filteredIssues.length === 0" class="table-empty-row">
              <td colspan="9">
                <div class="empty-state">暂无问题会话数据</div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div v-if="!loading" class="table-statusbar">
        <span>{{ filteredIssues.length === 0 ? "暂无分页" : "第 1 / 共 1 页" }}</span>
        <span>本页 {{ filteredIssues.length }} 条</span>
        <span>筛选后 {{ filteredIssues.length }} 条 / 共 {{ issues.length }} 条</span>
      </div>
    <div v-if="editingIssue" class="modal-overlay">
      <div class="modal-card modal-card--narrow">
        <header class="modal-header">
          <h3>处理问题会话</h3>
          <button class="btn-icon-only modal-close-button" title="关闭" @click="closeEdit">×</button>
        </header>

        <form class="modal-form" @submit.prevent="submitEdit">
          <div class="modal-body">
            <label class="field select-field">
              <span>状态</span>
              <select v-model="editForm.state">
                <option value="open">待处理</option>
                <option value="in_progress">处理中</option>
                <option value="resolved">已解决</option>
                <option value="closed">已关闭</option>
              </select>
            </label>
            <label class="field select-field">
              <span>优先级</span>
              <select v-model="editForm.priority">
                <option value="low">低</option>
                <option value="normal">普通</option>
                <option value="high">高</option>
                <option value="urgent">紧急</option>
              </select>
            </label>
            <label class="field">
              <span>处理备注</span>
              <textarea
                v-model="editForm.resolution"
                maxlength="4000"
                rows="5"
                placeholder="填写处理过程、解决方案或关闭原因"
              />
            </label>
          </div>

          <div class="modal-actions toolbar-actions">
            <button type="submit" class="btn btn-primary" :disabled="submitting">
              {{ submitting ? "保存中..." : "保存" }}
            </button>
            <button type="button" class="btn btn-ghost" :disabled="submitting" @click="closeEdit">取消</button>
          </div>
        </form>
      </div>
    </div>
  </section>
</template>

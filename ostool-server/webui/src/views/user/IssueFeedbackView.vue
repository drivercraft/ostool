<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { useRouter } from "vue-router";

import { api } from "@/api";
import Icon from "@/components/Icon.vue";
import StatusPill from "@/components/StatusPill.vue";
import { useAuthStore } from "@/stores/auth";
import { useUiStore } from "@/stores/ui";
import type { IssueSessionPriority, IssueSessionResponse, IssueSessionState } from "@/types/api";
import {
  getIssueCategoryLabel,
  getIssuePriorityLabel,
  getIssueStateDisplay,
} from "@/utils/issueSession";

const ui = useUiStore();
const auth = useAuthStore();
const router = useRouter();

const loading = ref(true);
const submitting = ref(false);
const search = ref("");
const stateFilter = ref<IssueSessionState | "all">("all");
const issues = ref<IssueSessionResponse[]>([]);
const form = reactive({
  title: "",
  category: "general",
  priority: "normal" as IssueSessionPriority,
  lease_id: "",
  session_id: "",
  description: "",
});

const filteredIssues = computed(() =>
  issues.value.filter(({ issue }) => {
    if (stateFilter.value !== "all" && issue.state !== stateFilter.value) {
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
      getIssueCategoryLabel(issue.category),
      issue.description,
      issue.lease_id ?? "",
      issue.session_id ?? "",
      issue.resolution ?? "",
    ].some((value) => value.toLowerCase().includes(query));
  }),
);

const canSubmit = computed(() =>
  form.title.trim().length > 0
    && form.title.trim().length <= 128
    && form.description.trim().length > 0
    && form.description.trim().length <= 4000,
);

function resetForm() {
  form.title = "";
  form.category = "general";
  form.priority = "normal";
  form.lease_id = "";
  form.session_id = "";
  form.description = "";
}

function formatDateTime(value: string | null) {
  return value ? new Date(value).toLocaleString("zh-CN", { hour12: false }) : "-";
}

async function loadIssues() {
  loading.value = true;
  try {
    const response = await api.user.listIssueSessions();
    issues.value = response.issues;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

async function submitIssue() {
  if (!canSubmit.value) {
    ui.setError("请填写问题标题和问题描述");
    return;
  }
  submitting.value = true;
  try {
    const created = await api.user.createIssueSession({
      title: form.title.trim(),
      category: form.category,
      priority: form.priority,
      lease_id: form.lease_id.trim() || null,
      session_id: form.session_id.trim() || null,
      description: form.description.trim(),
    });
    issues.value = [created, ...issues.value.filter((item) => item.issue.id !== created.issue.id)];
    resetForm();
    ui.setSuccess("问题反馈已提交");
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    submitting.value = false;
  }
}

onMounted(() => {
  ui.clearMessages();
  if (!auth.isAuthenticated) {
    void router.replace("/login");
    return;
  }
  void loadIssues();
});
</script>

<template>
  <section class="issue-feedback-page dashboard-rentals-section card">
    <section class="dashboard-subsection issue-feedback-submit-section">
      <div class="dashboard-subsection-head">
        <div>
          <h4>提交反馈</h4>
          <span>填写问题现象、关联租赁或会话，管理员会在后台处理。</span>
        </div>
      </div>

      <form class="issue-feedback-form" @submit.prevent="submitIssue">
        <label class="field is-required issue-field-full">
          <span>问题标题</span>
          <input
            v-model="form.title"
            type="text"
            maxlength="128"
            :disabled="submitting"
            placeholder="例如：串口会话无法连接"
          />
        </label>

        <label class="field select-field">
          <span>问题分类</span>
          <select v-model="form.category" :disabled="submitting">
            <option value="general">一般问题</option>
            <option value="resource">资源问题</option>
            <option value="lease">租赁问题</option>
            <option value="session">会话问题</option>
            <option value="account">账号问题</option>
            <option value="other">其他问题</option>
          </select>
        </label>

        <label class="field select-field">
          <span>优先级</span>
          <select v-model="form.priority" :disabled="submitting">
            <option value="low">低</option>
            <option value="normal">普通</option>
            <option value="high">高</option>
            <option value="urgent">紧急</option>
          </select>
        </label>

        <label class="field">
          <span>租赁 ID</span>
          <input
            v-model="form.lease_id"
            type="text"
            maxlength="128"
            :disabled="submitting"
            placeholder="可选，填写关联租赁 ID"
          />
        </label>

        <label class="field">
          <span>会话 ID</span>
          <input
            v-model="form.session_id"
            type="text"
            maxlength="128"
            :disabled="submitting"
            placeholder="可选，填写关联会话 ID"
          />
        </label>

        <label class="field is-required issue-field-full">
          <span>问题描述</span>
          <textarea
            v-model="form.description"
            maxlength="4000"
            rows="6"
            :disabled="submitting"
            placeholder="请描述复现步骤、期望结果、实际现象，以及相关日志或时间点。"
          />
        </label>

        <div class="dashboard-form-actions issue-field-full">
          <button class="btn btn-primary" type="submit" :disabled="!canSubmit || submitting">
            <Icon name="bell" :size="14" class="btn-icon" />
            {{ submitting ? "提交中..." : "提交反馈" }}
          </button>
          <button class="btn btn-ghost" type="button" :disabled="submitting" @click="resetForm">
            清空
          </button>
        </div>
      </form>
    </section>

    <section class="dashboard-subsection">
      <div class="dashboard-subsection-head">
        <div>
          <h4>我的反馈</h4>
          <span>{{ filteredIssues.length }} 条反馈</span>
        </div>
        <div class="issue-feedback-filters">
          <label class="search-field">
            <Icon name="search" :size="16" />
            <input v-model="search" type="search" maxlength="128" placeholder="搜索标题 / 租赁 / 会话" />
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
          <button class="btn btn-ghost btn-sm" type="button" :disabled="loading" @click="loadIssues">
            刷新
          </button>
        </div>
      </div>

      <div v-if="loading" class="empty-state"><div class="empty-state-icon">&#9641;</div>正在加载反馈...</div>
      <div v-else-if="filteredIssues.length === 0" class="empty-state">
        <div class="empty-state-icon">&#9641;</div>
        暂无问题反馈数据
      </div>

      <div v-else class="issue-feedback-list">
        <article v-for="item in filteredIssues" :key="item.issue.id" class="issue-feedback-card">
          <header class="issue-feedback-card-head">
            <div>
              <strong>{{ item.issue.title }}</strong>
              <span>{{ getIssueCategoryLabel(item.issue.category) }} · {{ getIssuePriorityLabel(item.issue.priority) }}</span>
            </div>
            <StatusPill
              :tone="getIssueStateDisplay(item.issue.state).tone"
              :label="getIssueStateDisplay(item.issue.state).label"
            />
          </header>

          <p>{{ item.issue.description }}</p>

          <dl class="key-value-list issue-feedback-meta">
            <dt>反馈 ID</dt>
            <dd>{{ item.issue.id }}</dd>
            <dt>租赁 ID</dt>
            <dd>{{ item.issue.lease_id || "-" }}</dd>
            <dt>会话 ID</dt>
            <dd>{{ item.issue.session_id || "-" }}</dd>
            <dt>提交时间</dt>
            <dd>{{ formatDateTime(item.issue.created_at) }}</dd>
            <dt>更新时间</dt>
            <dd>{{ formatDateTime(item.issue.updated_at) }}</dd>
          </dl>

          <div v-if="item.issue.resolution" class="issue-feedback-resolution">
            <span>处理备注</span>
            <p>{{ item.issue.resolution }}</p>
          </div>
        </article>
      </div>
    </section>
  </section>
</template>

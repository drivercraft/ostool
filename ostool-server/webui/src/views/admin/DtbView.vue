<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import Icon from "@/components/Icon.vue";
import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { DtbFileResponse } from "@/types/api";

const ui = useUiStore();
const loading = ref(true);
const creating = ref(false);
const updatingName = ref<string | null>(null);
const deletingName = ref<string | null>(null);
const dtbs = ref<DtbFileResponse[]>([]);
const creatingDtb = ref(false);
const newDtbName = ref("");
const newDtbArchitecture = ref("");
const newDtbCompatible = ref("");
const newDtbDescription = ref("");
const newDtbFile = ref<File | null>(null);
const newDtbInput = ref<HTMLInputElement | null>(null);
const editingDtbName = ref<string | null>(null);
const editDtbName = ref("");
const editDtbArchitecture = ref("");
const editDtbCompatible = ref("");
const editDtbDescription = ref("");
const editDtbFile = ref<File | null>(null);
const editDtbFileInput = ref<HTMLInputElement | null>(null);
const pointerDownOnModalOverlay = ref(false);
const search = ref("");
const sizeFilter = ref<"all" | "small" | "medium" | "large">("all");
const DTB_UPLOAD_MAX_MIB = 10;
const DTB_UPLOAD_MAX_BYTES = DTB_UPLOAD_MAX_MIB * 1024 * 1024;

const filteredDtbs = computed(() => {
  const query = search.value.trim().toLowerCase();
  return dtbs.value.filter((dtb) =>
    matchesDtbSize(dtb) && matchesDtbSearch(dtb, query),
  );
});

function matchesDtbSize(dtb: DtbFileResponse) {
  if (sizeFilter.value === "small") {
    return dtb.size <= 64 * 1024;
  }
  if (sizeFilter.value === "medium") {
    return dtb.size > 64 * 1024 && dtb.size <= 1024 * 1024;
  }
  if (sizeFilter.value === "large") {
    return dtb.size > 1024 * 1024;
  }
  return true;
}

function matchesDtbSearch(dtb: DtbFileResponse, query: string) {
  if (!query) {
    return true;
  }
  return [
    dtb.name,
    dtb.relative_tftp_path_template,
    dtb.boot_architecture ?? "",
    dtb.compatible ?? "",
    dtb.description ?? "",
    dtb.sha256 ?? "",
  ].some((value) => value.toLowerCase().includes(query));
}

function formatSize(size: number): string {
  if (size < 1024) {
    return `${size} B`;
  }
  if (size < 1024 * 1024) {
    return `${(size / 1024).toFixed(1)} KiB`;
  }
  return `${(size / (1024 * 1024)).toFixed(1)} MiB`;
}

function formatTime(value: string): string {
  return new Date(value).toLocaleString("zh-CN", { hour12: false });
}

function validateDtbFileSize(file: File | null): string | null {
  if (file && file.size > DTB_UPLOAD_MAX_BYTES) {
    return `DTB 文件大小不能超过 ${DTB_UPLOAD_MAX_MIB} MiB`;
  }
  return null;
}

async function loadDtbs() {
  loading.value = true;
  try {
    const files = await api.listDtbs();
    dtbs.value = files;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

function onNewFileChange(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0] ?? null;
  newDtbFile.value = file;
  if (file) {
    newDtbName.value = file.name;
  }
}

function dtbMetadata(architecture: string, compatible: string, description: string) {
  return {
    boot_architecture: architecture.trim() || null,
    compatible: compatible.trim() || null,
    description: description.trim() || null,
  };
}

function resetCreateForm() {
  newDtbName.value = "";
  newDtbArchitecture.value = "";
  newDtbCompatible.value = "";
  newDtbDescription.value = "";
  newDtbFile.value = null;
  if (newDtbInput.value) {
    newDtbInput.value.value = "";
  }
}

function openCreateDtb() {
  resetCreateForm();
  creatingDtb.value = true;
}

function closeCreateDtb() {
  creatingDtb.value = false;
  resetCreateForm();
}

function onReplaceFileChange(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0] ?? null;
  editDtbFile.value = file;
  if (file) {
    editDtbName.value = file.name;
  }
}

async function createDtb() {
  if (!newDtbFile.value) {
    ui.setError("请选择要上传的 DTB 文件");
    return;
  }
  const sizeError = validateDtbFileSize(newDtbFile.value);
  if (sizeError) {
    ui.setError(sizeError);
    return;
  }
  const name = newDtbName.value.trim() || newDtbFile.value.name;
  if (!name) {
    ui.setError("请填写 DTB 文件名");
    return;
  }

  creating.value = true;
  try {
    await api.createDtb(
      name,
      newDtbFile.value,
      dtbMetadata(newDtbArchitecture.value, newDtbCompatible.value, newDtbDescription.value),
    );
    closeCreateDtb();
    ui.setSuccess(`已上传 DTB ${name}`);
    await loadDtbs();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    creating.value = false;
  }
}

function openEditDtb(dtb: DtbFileResponse) {
  editingDtbName.value = dtb.name;
  editDtbName.value = dtb.name;
  editDtbArchitecture.value = dtb.boot_architecture ?? "";
  editDtbCompatible.value = dtb.compatible ?? "";
  editDtbDescription.value = dtb.description ?? "";
  editDtbFile.value = null;
  if (editDtbFileInput.value) {
    editDtbFileInput.value.value = "";
  }
}

function closeEditDtb() {
  editingDtbName.value = null;
  editDtbName.value = "";
  editDtbArchitecture.value = "";
  editDtbCompatible.value = "";
  editDtbDescription.value = "";
  editDtbFile.value = null;
  if (editDtbFileInput.value) {
    editDtbFileInput.value.value = "";
  }
}

function onModalOverlayPointerDown(event: PointerEvent) {
  pointerDownOnModalOverlay.value = event.target === event.currentTarget;
}

function onModalOverlayClick(event: MouseEvent) {
  if (pointerDownOnModalOverlay.value && event.target === event.currentTarget) {
    if (creatingDtb.value) {
      closeCreateDtb();
    } else {
      closeEditDtb();
    }
  }
  pointerDownOnModalOverlay.value = false;
}

async function saveDtb() {
  const currentName = editingDtbName.value;
  if (!currentName) {
    return;
  }
  const nextName = editDtbName.value.trim();
  const replaceFile = editDtbFile.value;
  const sizeError = validateDtbFileSize(replaceFile);
  if (sizeError) {
    ui.setError(sizeError);
    return;
  }
  if (!nextName) {
    ui.setError("DTB 文件名不能为空");
    return;
  }
  const metadata = dtbMetadata(
    editDtbArchitecture.value,
    editDtbCompatible.value,
    editDtbDescription.value,
  );
  if (
    nextName === currentName
    && !replaceFile
    && JSON.stringify(metadata) === JSON.stringify(dtbMetadata("", "", ""))
  ) {
    ui.setError("请修改文件名、元信息或选择新的 DTB 文件");
    return;
  }

  updatingName.value = currentName;
  try {
    const updated = await api.updateDtb(
      currentName,
      nextName === currentName ? null : nextName,
      replaceFile,
      metadata,
    );
    ui.setSuccess(`已更新 DTB ${updated.name}`);
    closeEditDtb();
    await loadDtbs();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    updatingName.value = null;
  }
}

async function removeDtb(name: string) {
  const confirmed = await ui.confirm({
    tone: "danger",
    title: "删除 DTB",
    message: `确认删除 DTB ${name} 吗？`,
    confirmLabel: "删除",
  });
  if (!confirmed) {
    return;
  }

  deletingName.value = name;
  try {
    await api.deleteDtb(name);
    ui.setSuccess(`已删除 DTB ${name}`);
    await loadDtbs();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    deletingName.value = null;
  }
}

onMounted(() => {
  ui.clearMessages();
  void loadDtbs();
});
</script>

<template>
  <section class="page-grid admin-list-page dtb-page">
    <div class="panel admin-table-panel">
      <div class="admin-toolbar">
        <div class="admin-toolbar-left">
          <button class="btn btn-primary" @click="openCreateDtb">
            <Icon name="plus" :size="16" class="btn-icon" />
            上传 DTB
          </button>
          <button class="btn btn-ghost btn-sm" @click="loadDtbs">刷新</button>
        </div>
        <div class="admin-toolbar-right">
          <label class="search-field">
            <Icon name="search" :size="16" />
            <input v-model="search" type="search" placeholder="搜索名称 / 架构 / compatible / 路径" />
          </label>
          <label class="field filter-field">
            <span>大小</span>
            <select v-model="sizeFilter">
              <option value="all">全部大小</option>
              <option value="small">小于 64 KiB</option>
              <option value="medium">64 KiB - 1 MiB</option>
              <option value="large">大于 1 MiB</option>
            </select>
          </label>
        </div>
      </div>

      <div v-if="loading" class="empty-state">正在加载 DTB 列表...</div>
      <div v-else-if="dtbs.length === 0" class="empty-state">当前还没有上传任何 DTB。</div>
      <div v-else-if="filteredDtbs.length === 0" class="empty-state">没有符合条件的 DTB。</div>
      <div v-else class="table-scroll">
        <table class="data-table">
          <thead>
            <tr>
              <th class="col-index">序号</th>
              <th>名称</th>
              <th>架构</th>
              <th>Compatible</th>
              <th>大小</th>
              <th>更新时间</th>
              <th>说明</th>
              <th class="col-actions">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(dtb, index) in filteredDtbs" :key="dtb.name">
              <td class="col-index">{{ index + 1 }}</td>
              <td>
                <div class="dtb-name-cell">
                  <code>{{ dtb.name }}</code>
                  <span>{{ dtb.relative_tftp_path_template }}</span>
                </div>
              </td>
              <td>{{ dtb.boot_architecture || "-" }}</td>
              <td><code>{{ dtb.compatible || "-" }}</code></td>
              <td>{{ formatSize(dtb.size) }}</td>
              <td>{{ formatTime(dtb.updated_at) }}</td>
              <td class="muted">{{ dtb.description || "无说明" }}</td>
              <td class="col-actions">
                <div class="row-actions">
                  <button
                    class="btn-icon-only"
                    title="编辑"
                    :disabled="updatingName === dtb.name"
                    @click="openEditDtb(dtb)"
                  >
                    <Icon name="edit" :size="16" />
                  </button>
                  <button
                    class="btn-icon-only"
                    title="删除"
                    :disabled="deletingName === dtb.name"
                    @click="removeDtb(dtb.name)"
                  >
                    <Icon name="trash" :size="16" />
                  </button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div v-if="!loading" class="table-statusbar">
        <span>{{ filteredDtbs.length === 0 ? "暂无分页" : "第 1 / 共 1 页" }}</span>
        <span>本页 {{ filteredDtbs.length }} 条</span>
        <span>筛选后 {{ filteredDtbs.length }} 条 / 共 {{ dtbs.length }} 条</span>
      </div>
    </div>
  </section>

  <div
    v-if="creatingDtb"
    class="modal-overlay"
    @pointerdown="onModalOverlayPointerDown"
    @click="onModalOverlayClick"
  >
    <div class="modal-card modal-card--dtb-form">
      <header class="modal-header">
        <div>
          <h3>上传 DTB</h3>
          <p class="muted">补充架构、compatible 和说明，方便后续按板型选择。</p>
        </div>
        <button class="btn-icon-only modal-close-button" title="关闭" @click="closeCreateDtb">×</button>
      </header>

      <form class="modal-form" @submit.prevent="createDtb">
        <div class="modal-body modal-body-grid">
          <label class="field">
            <span>文件名</span>
            <input v-model="newDtbName" placeholder="例如 rk3568-evb.dtb" />
          </label>
          <label class="field">
            <span>架构描述</span>
            <input v-model="newDtbArchitecture" placeholder="例如 arm64 / riscv64" />
          </label>
          <label class="field modal-field-full">
            <span>Compatible</span>
            <input v-model="newDtbCompatible" placeholder="例如 rockchip,rk3568-evb" />
          </label>
          <label class="field modal-field-full">
            <span>说明</span>
            <textarea v-model="newDtbDescription" placeholder="记录适用开发板、内核版本或维护说明" />
          </label>
          <label class="field modal-field-full">
            <span>DTB 文件</span>
            <input
              ref="newDtbInput"
              type="file"
              accept=".dtb,application/octet-stream"
              @change="onNewFileChange"
            />
          </label>
          <p class="modal-hint muted">单个 DTB 文件最大支持 {{ DTB_UPLOAD_MAX_MIB }} MiB。</p>
        </div>

        <div class="modal-actions">
          <button type="submit" class="btn btn-primary" :disabled="creating">
            {{ creating ? "上传中..." : "上传 DTB" }}
          </button>
          <button type="button" class="btn btn-ghost" :disabled="creating" @click="closeCreateDtb">取消</button>
        </div>
      </form>
    </div>
  </div>

  <div
    v-if="editingDtbName"
    class="modal-overlay"
    @pointerdown="onModalOverlayPointerDown"
    @click="onModalOverlayClick"
  >
    <div class="modal-card modal-card--dtb-form">
      <header class="modal-header">
        <div>
          <h3>{{ editingDtbName }}</h3>
          <p class="muted">修改 DTB 名称、元信息，或选择文件替换内容。</p>
        </div>
        <button class="btn-icon-only modal-close-button" title="关闭" @click="closeEditDtb">×</button>
      </header>

      <form class="modal-form" @submit.prevent="saveDtb">
        <div class="modal-body modal-body-grid">
          <label class="field">
            <span>文件名</span>
            <input v-model="editDtbName" placeholder="例如 rk3568-evb.dtb" />
          </label>
          <label class="field">
            <span>架构描述</span>
            <input v-model="editDtbArchitecture" placeholder="例如 arm64 / riscv64" />
          </label>
          <label class="field modal-field-full">
            <span>Compatible</span>
            <input v-model="editDtbCompatible" placeholder="例如 rockchip,rk3568-evb" />
          </label>
          <label class="field modal-field-full">
            <span>说明</span>
            <textarea v-model="editDtbDescription" placeholder="记录适用开发板、内核版本或维护说明" />
          </label>
          <label class="field modal-field-full">
            <span>替换文件</span>
            <input
              ref="editDtbFileInput"
              type="file"
              accept=".dtb,application/octet-stream"
              @change="onReplaceFileChange"
            />
          </label>
          <p class="modal-hint muted">替换上传时同样受 {{ DTB_UPLOAD_MAX_MIB }} MiB 限制。</p>
        </div>

        <div class="modal-actions">
          <button type="submit" class="btn btn-primary" :disabled="updatingName === editingDtbName">
            {{ updatingName === editingDtbName ? "保存中..." : "保存修改" }}
          </button>
          <button type="button" class="btn btn-ghost" :disabled="updatingName === editingDtbName" @click="closeEditDtb">
            取消
          </button>
        </div>
      </form>
    </div>
  </div>
</template>

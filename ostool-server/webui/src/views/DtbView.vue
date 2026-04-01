<script setup lang="ts">
import { onMounted, ref } from "vue";

import { api } from "@/api/client";
import { useUiStore } from "@/stores/ui";
import type { DtbFileResponse } from "@/types/api";

const ui = useUiStore();
const loading = ref(true);
const creating = ref(false);
const updatingName = ref<string | null>(null);
const deletingName = ref<string | null>(null);
const dtbs = ref<DtbFileResponse[]>([]);
const renameDrafts = ref<Record<string, string>>({});
const replaceFiles = ref<Record<string, File | null>>({});
const newDtbName = ref("");
const newDtbFile = ref<File | null>(null);
const newDtbInput = ref<HTMLInputElement | null>(null);

function hydrateDrafts(files: DtbFileResponse[]) {
  renameDrafts.value = Object.fromEntries(files.map((file) => [file.name, file.name]));
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

async function loadDtbs() {
  loading.value = true;
  try {
    const files = await api.listDtbs();
    dtbs.value = files;
    hydrateDrafts(files);
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

function onReplaceFileChange(name: string, event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0] ?? null;
  replaceFiles.value = {
    ...replaceFiles.value,
    [name]: file,
  };
  if (file) {
    renameDrafts.value = {
      ...renameDrafts.value,
      [name]: file.name,
    };
  }
}

async function createDtb() {
  if (!newDtbFile.value) {
    ui.setError("请选择要上传的 DTB 文件");
    return;
  }
  const name = newDtbName.value.trim() || newDtbFile.value.name;
  if (!name) {
    ui.setError("请填写 DTB 文件名");
    return;
  }

  creating.value = true;
  try {
    await api.createDtb(name, newDtbFile.value);
    newDtbName.value = "";
    newDtbFile.value = null;
    if (newDtbInput.value) {
      newDtbInput.value.value = "";
    }
    ui.setSuccess(`已上传 DTB ${name}`);
    await loadDtbs();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    creating.value = false;
  }
}

async function saveDtb(currentName: string) {
  const nextName = (renameDrafts.value[currentName] ?? currentName).trim();
  const replaceFile = replaceFiles.value[currentName] ?? null;
  if (!nextName) {
    ui.setError("DTB 文件名不能为空");
    return;
  }
  if (nextName === currentName && !replaceFile) {
    ui.setError("请修改文件名或选择新的 DTB 文件");
    return;
  }

  updatingName.value = currentName;
  try {
    const updated = await api.updateDtb(
      currentName,
      nextName === currentName ? null : nextName,
      replaceFile,
    );
    ui.setSuccess(`已更新 DTB ${updated.name}`);
    replaceFiles.value = {
      ...replaceFiles.value,
      [currentName]: null,
      [updated.name]: null,
    };
    await loadDtbs();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    updatingName.value = null;
  }
}

async function removeDtb(name: string) {
  if (!window.confirm(`确认删除 DTB ${name} 吗？`)) {
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
  <section class="page-grid">
    <div class="panel">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">独立 DTB 仓库</p>
          <h3>DTB 管理</h3>
        </div>
        <div class="toolbar-actions">
          <button class="ghost-button" @click="loadDtbs">刷新</button>
        </div>
      </div>

      <section class="form-section">
        <h4>上传新 DTB</h4>
        <div class="form-grid three-columns">
          <label class="field">
            <span>文件名</span>
            <input v-model="newDtbName" placeholder="例如 board.dtb" />
          </label>
          <label class="field">
            <span>文件</span>
            <input
              ref="newDtbInput"
              type="file"
              accept=".dtb,application/octet-stream"
              @change="onNewFileChange"
            />
          </label>
          <div class="field action-field">
            <span>操作</span>
            <button class="primary-button" :disabled="creating" @click="createDtb">
              {{ creating ? "上传中..." : "上传 DTB" }}
            </button>
          </div>
        </div>
      </section>

      <div v-if="loading" class="empty-state">正在加载 DTB 列表...</div>
      <div v-else-if="dtbs.length === 0" class="empty-state">当前还没有上传任何 DTB。</div>
      <table v-else class="data-table">
        <thead>
          <tr>
            <th>名称</th>
            <th>大小</th>
            <th>更新时间</th>
            <th>TFTP 路径模板</th>
            <th>重命名 / 替换</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="dtb in dtbs" :key="dtb.name">
            <td><code>{{ dtb.name }}</code></td>
            <td>{{ formatSize(dtb.size) }}</td>
            <td>{{ formatTime(dtb.updated_at) }}</td>
            <td><code>{{ dtb.relative_tftp_path_template }}</code></td>
            <td>
              <div class="table-form-stack">
                <input
                  v-model="renameDrafts[dtb.name]"
                  :placeholder="dtb.name"
                />
                <input
                  type="file"
                  accept=".dtb,application/octet-stream"
                  @change="onReplaceFileChange(dtb.name, $event)"
                />
              </div>
            </td>
            <td>
              <div class="toolbar-actions">
                <button
                  class="ghost-button compact-button"
                  :disabled="updatingName === dtb.name"
                  @click="saveDtb(dtb.name)"
                >
                  {{ updatingName === dtb.name ? "保存中..." : "保存" }}
                </button>
                <button
                  class="danger-button compact-button"
                  :disabled="deletingName === dtb.name"
                  @click="removeDtb(dtb.name)"
                >
                  {{ deletingName === dtb.name ? "删除中..." : "删除" }}
                </button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>

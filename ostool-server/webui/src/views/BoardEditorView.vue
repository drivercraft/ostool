<script setup lang="ts">
import { JsonForms, type JsonFormsChangeEvent } from "@jsonforms/vue";
import { vanillaRenderers } from "@jsonforms/vue-vanilla";
import { computed, markRaw, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";

import { api } from "@/api/client";
import { useUiStore } from "@/stores/ui";
import type { BoardEditorData, BoardEditorDocument } from "@/types/api";
import { jsonFormsAjv } from "@/utils/jsonFormsAjv";
import { boardEditorUiSchema } from "@/utils/boardEditorUiSchema";

const renderers = markRaw(vanillaRenderers);

const route = useRoute();
const router = useRouter();
const ui = useUiStore();

const loading = ref(true);
const saving = ref(false);
const deleting = ref(false);
const validationError = ref("");
const document = ref<BoardEditorDocument | null>(null);
const isEditing = computed(() => typeof route.params.boardId === "string");
const boardId = computed(() => route.params.boardId as string | undefined);

function formatValidationErrors(errors: JsonFormsChangeEvent["errors"]): string {
  return (errors ?? [])
    .map((error) => {
      const path = error.instancePath || error.schemaPath || "表单";
      return error.message ? `${path}: ${error.message}` : path;
    })
    .join("\n");
}

function updateDocumentData(data: BoardEditorData) {
  if (!document.value) {
    return;
  }
  document.value = {
    ...document.value,
    data,
  };
}

function onFormChange(event: JsonFormsChangeEvent) {
  updateDocumentData(event.data as BoardEditorData);
  validationError.value = formatValidationErrors(event.errors);
}

async function loadEditor() {
  loading.value = true;
  ui.clearMessages();
  validationError.value = "";

  try {
    document.value = isEditing.value && boardId.value
      ? await api.getBoardEditor(boardId.value)
      : await api.getNewBoardEditor();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

async function saveBoard() {
  if (!document.value) {
    return;
  }
  if (validationError.value) {
    return;
  }

  saving.value = true;
  try {
    const saved = isEditing.value && boardId.value
      ? await api.updateBoard(boardId.value, document.value)
      : await api.createBoard(document.value);
    document.value = saved;
    ui.setSuccess(`已保存开发板 ${saved.data.name}`);
    await router.push(`/boards/${saved.data.id}`);
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    saving.value = false;
  }
}

async function removeBoard() {
  if (!boardId.value) {
    return;
  }
  if (!window.confirm(`确认删除开发板 ${boardId.value} 吗？`)) {
    return;
  }

  deleting.value = true;
  try {
    await api.deleteBoard(boardId.value);
    ui.setSuccess(`已删除开发板 ${boardId.value}`);
    await router.push("/boards");
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    deleting.value = false;
  }
}

onMounted(() => {
  void loadEditor();
});
</script>

<template>
  <section class="page-grid">
    <div class="panel">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">{{ isEditing ? "编辑现有开发板" : "创建新开发板" }}</p>
          <h3>{{ isEditing ? "开发板配置" : "新建开发板" }}</h3>
        </div>
        <div class="toolbar-actions">
          <button class="ghost-button" @click="loadEditor">刷新</button>
          <button class="primary-button" :disabled="saving || !document" @click="saveBoard">
            {{ saving ? "保存中..." : "保存配置" }}
          </button>
        </div>
      </div>

      <div v-if="loading" class="empty-state">正在加载开发板配置...</div>
      <template v-else-if="document">
        <p v-if="validationError" class="diagnostic-error">{{ validationError }}</p>

        <div class="board-editor-jsonforms">
          <JsonForms
            :data="document.data"
            :schema="document.schema"
            :uischema="boardEditorUiSchema"
            :renderers="renderers"
            :ajv="jsonFormsAjv"
            validation-mode="ValidateAndShow"
            @change="onFormChange"
          />
        </div>

        <div class="danger-zone" v-if="isEditing">
          <h4>危险操作</h4>
          <p>删除会移除对应的单板配置文件，且需要先释放占用该板的 session。</p>
          <button class="danger-button" :disabled="deleting" @click="removeBoard">
            {{ deleting ? "删除中..." : "删除开发板" }}
          </button>
        </div>
      </template>
    </div>
  </section>
</template>

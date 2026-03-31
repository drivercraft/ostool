<script setup lang="ts">
import { onMounted, ref } from "vue";

import { api } from "@/api/client";
import { useUiStore } from "@/stores/ui";
import type { AdminServerConfigResponse } from "@/types/api";

const ui = useUiStore();
const loading = ref(true);
const saving = ref(false);
const config = ref<AdminServerConfigResponse | null>(null);

async function loadConfig() {
  loading.value = true;
  try {
    config.value = await api.getServerConfig();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

async function saveConfig() {
  if (!config.value) {
    return;
  }

  saving.value = true;
  try {
    config.value = await api.updateServerConfig({
      lease: config.value.editable.lease,
    });
    ui.setSuccess("已保存 Server 安全配置");
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    saving.value = false;
  }
}

onMounted(() => {
  ui.clearMessages();
  void loadConfig();
});
</script>

<template>
  <section class="page-grid">
    <div class="panel">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">安全配置优先</p>
          <h3>Server 顶层配置</h3>
        </div>
        <div class="toolbar-actions">
          <button class="ghost-button" @click="loadConfig">刷新</button>
          <button class="primary-button" :disabled="saving || !config" @click="saveConfig">
            {{ saving ? "保存中..." : "保存配置" }}
          </button>
        </div>
      </div>

      <div v-if="loading" class="empty-state">正在加载 server 配置...</div>
      <template v-else-if="config">
        <div class="split-grid">
          <section class="panel nested-panel">
            <div class="panel-heading compact">
              <h4>只读信息</h4>
            </div>
            <dl class="key-value-list">
              <div>
                <dt>监听地址</dt>
                <dd>{{ config.readonly.listen_addr }}</dd>
              </div>
              <div>
                <dt>数据目录</dt>
                <dd>{{ config.readonly.data_dir }}</dd>
              </div>
              <div>
                <dt>板子目录</dt>
                <dd>{{ config.readonly.board_dir }}</dd>
              </div>
            </dl>
          </section>

          <section class="panel nested-panel">
            <div class="panel-heading compact">
              <h4>可编辑 Lease 参数</h4>
            </div>
            <div class="form-grid">
              <label class="field">
                <span>默认 TTL（秒）</span>
                <input v-model.number="config.editable.lease.default_ttl_secs" type="number" min="1" />
              </label>
              <label class="field">
                <span>最大 TTL（秒）</span>
                <input v-model.number="config.editable.lease.max_ttl_secs" type="number" min="1" />
              </label>
              <label class="field">
                <span>GC 间隔（秒）</span>
                <input v-model.number="config.editable.lease.gc_interval_secs" type="number" min="1" />
              </label>
            </div>
            <p class="muted">
              `lease` 保存后立即生效；`listen_addr`、`data_dir`、`board_dir` 仍保持只读，避免运行中修改导致服务行为不稳定。
            </p>
          </section>
        </div>
      </template>
    </div>
  </section>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import { api } from "@/api/client";
import { useUiStore } from "@/stores/ui";
import type { AdminServerConfigResponse, NetworkInterfaceSummary } from "@/types/api";

const ui = useUiStore();
const loading = ref(true);
const saving = ref(false);
const config = ref<AdminServerConfigResponse | null>(null);
const networkInterfaces = ref<NetworkInterfaceSummary[]>([]);
const networkInterfaceOptions = computed(() => {
  const options = [...networkInterfaces.value];
  const currentInterface = config.value?.editable.network.interface.trim() ?? "";
  if (currentInterface && !options.some((item) => item.name === currentInterface)) {
    options.unshift({
      name: currentInterface,
      label: `${currentInterface} (当前配置，未检测到)`,
      ipv4_addresses: [],
      netmask: null,
      loopback: false,
    });
  }
  return options;
});

async function loadConfig() {
  loading.value = true;
  try {
    const [serverConfig, interfaces] = await Promise.all([
      api.getServerConfig(),
      api.listNetworkInterfaces(),
    ]);
    config.value = serverConfig;
    networkInterfaces.value = interfaces;
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

  const sessionFileMaxMib = Number(config.value.editable.upload_limits.session_file_max_mib);
  if (!Number.isFinite(sessionFileMaxMib) || sessionFileMaxMib < 1) {
    ui.setError("Session 文件上传上限必须是大于等于 1 的整数 MiB");
    return;
  }
  config.value.editable.upload_limits.session_file_max_mib = Math.trunc(sessionFileMaxMib);

  saving.value = true;
  try {
    config.value = await api.updateServerConfig({
      network: config.value.editable.network,
      upload_limits: config.value.editable.upload_limits,
    });
    ui.setSuccess("已保存 Server 安全配置");
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    saving.value = false;
  }
}

async function refreshNetworkInterfaces() {
  try {
    networkInterfaces.value = await api.listNetworkInterfaces();
    ui.setSuccess("已刷新网络接口列表");
  } catch (error) {
    ui.setError((error as Error).message);
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
              <div>
                <dt>DTB 目录</dt>
                <dd>{{ config.readonly.dtb_dir }}</dd>
              </div>
              <div>
                <dt>DTB 上传上限</dt>
                <dd>{{ config.readonly.dtb_upload_max_mib }} MiB</dd>
              </div>
            </dl>
          </section>

          <section class="panel nested-panel">
            <div class="panel-heading compact">
              <h4>服务级网络配置</h4>
            </div>
            <div class="form-grid">
              <label class="field">
                <span>网络接口</span>
                <div class="inline-field-group">
                  <select v-model="config.editable.network.interface">
                    <option value="">自动选择第一个非 loopback 接口</option>
                    <option
                      v-for="networkInterface in networkInterfaceOptions"
                      :key="networkInterface.name"
                      :value="networkInterface.name"
                    >
                      {{ networkInterface.label }}
                    </option>
                  </select>
                  <button class="ghost-button compact-button" type="button" @click="refreshNetworkInterfaces">
                    刷新网卡
                  </button>
                </div>
              </label>
              <label class="field">
                <span>Session 文件上传上限</span>
                <input
                  v-model.number="config.editable.upload_limits.session_file_max_mib"
                  type="number"
                  min="1"
                  step="1"
                />
              </label>
            </div>
            <p class="muted">
              服务级网络配置和 session 文件上传上限保存后会立即作用于新的请求；DTB 上传上限固定为
              {{ config.readonly.dtb_upload_max_mib }} MiB。
            </p>
            <p class="muted">
              `listen_addr`、`data_dir`、`board_dir`、`dtb_dir` 仍保持只读，避免运行中修改导致服务行为不稳定。
            </p>
          </section>
        </div>
      </template>
    </div>
  </section>
</template>

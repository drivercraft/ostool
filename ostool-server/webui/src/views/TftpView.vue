<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api/client";
import { useUiStore } from "@/stores/ui";
import type { BuiltinTftpConfig, SystemTftpdHpaConfig, TftpConfig, TftpStatus } from "@/types/api";
import { describeTftpStatus } from "@/utils/tftpStatus";

const ui = useUiStore();
const loading = ref(true);
const saving = ref(false);
const reconciling = ref(false);
const tftpConfig = ref<TftpConfig | null>(null);
const tftpStatus = ref<TftpStatus | null>(null);

const tone = computed(() =>
  tftpStatus.value
    ? describeTftpStatus(tftpStatus.value)
    : { tone: "neutral" as const, label: "未知" },
);

async function loadTftp() {
  loading.value = true;
  try {
    const [configResponse, statusResponse] = await Promise.all([api.getTftpConfig(), api.getTftpStatus()]);
    tftpConfig.value = configResponse.tftp;
    tftpStatus.value = statusResponse.status;
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

function switchProvider(provider: TftpConfig["provider"]) {
  if (provider === "builtin") {
    const current = tftpConfig.value;
    const next: BuiltinTftpConfig = {
      provider: "builtin",
      enabled: current?.enabled ?? true,
      root_dir: current?.root_dir ?? "/srv/tftp",
      bind_addr: current && current.provider === "builtin" ? current.bind_addr : "0.0.0.0:69",
    };
    tftpConfig.value = next;
    return;
  }

  const current = tftpConfig.value;
  const next: SystemTftpdHpaConfig = {
    provider: "system_tftpd_hpa",
    enabled: current?.enabled ?? true,
    root_dir: current?.root_dir ?? "/srv/tftp",
    config_path:
      current && current.provider === "system_tftpd_hpa"
        ? current.config_path
        : "/etc/default/tftpd-hpa",
    service_name:
      current && current.provider === "system_tftpd_hpa" ? current.service_name : "tftpd-hpa",
    username: current && current.provider === "system_tftpd_hpa" ? current.username : "tftp",
    address: current && current.provider === "system_tftpd_hpa" ? current.address : ":69",
    options: current && current.provider === "system_tftpd_hpa" ? current.options : "-l -s -c",
    manage_config:
      current && current.provider === "system_tftpd_hpa" ? current.manage_config : false,
    reconcile_on_start:
      current && current.provider === "system_tftpd_hpa" ? current.reconcile_on_start : false,
  };
  tftpConfig.value = next;
}

async function saveConfig() {
  if (!tftpConfig.value) {
    return;
  }

  saving.value = true;
  try {
    const response = await api.updateTftpConfig(tftpConfig.value);
    tftpConfig.value = response.tftp;
    ui.setSuccess("已保存 TFTP 配置");
    await loadTftp();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    saving.value = false;
  }
}

async function reconcile() {
  reconciling.value = true;
  try {
    const response = await api.reconcileTftp();
    tftpStatus.value = response.status;
    ui.setSuccess("已执行 TFTP reconcile");
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    reconciling.value = false;
  }
}

onMounted(() => {
  ui.clearMessages();
  void loadTftp();
});
</script>

<template>
  <section class="page-grid">
    <div class="panel">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">Provider 与运行状态</p>
          <h3>TFTP 配置管理</h3>
        </div>
        <div class="toolbar-actions">
          <button class="ghost-button" @click="loadTftp">刷新</button>
          <button class="ghost-button" :disabled="reconciling" @click="reconcile">
            {{ reconciling ? "执行中..." : "执行 Reconcile" }}
          </button>
          <button class="primary-button" :disabled="saving || !tftpConfig" @click="saveConfig">
            {{ saving ? "保存中..." : "保存配置" }}
          </button>
        </div>
      </div>

      <div v-if="loading" class="empty-state">正在加载 TFTP 配置...</div>
      <template v-else-if="tftpConfig && tftpStatus">
        <div class="split-grid">
          <section class="panel nested-panel">
            <div class="panel-heading compact">
              <h4>当前 Provider</h4>
              <StatusPill :tone="tone.tone" :label="tone.label" />
            </div>
            <div class="form-grid two-columns">
              <label class="field">
                <span>Provider</span>
                <select :value="tftpConfig.provider" @change="switchProvider(($event.target as HTMLSelectElement).value as TftpConfig['provider'])">
                  <option value="builtin">builtin</option>
                  <option value="system_tftpd_hpa">system_tftpd_hpa</option>
                </select>
              </label>
              <label class="checkbox-field">
                <input v-model="tftpConfig.enabled" type="checkbox" />
                <span>启用 TFTP</span>
              </label>
              <label class="field">
                <span>根目录</span>
                <input v-model="tftpConfig.root_dir" />
              </label>
              <template v-if="tftpConfig.provider === 'builtin'">
                <label class="field">
                  <span>绑定地址</span>
                  <input v-model="tftpConfig.bind_addr" placeholder="0.0.0.0:69" />
                </label>
              </template>
              <template v-else>
                <label class="field">
                  <span>配置文件</span>
                  <input v-model="tftpConfig.config_path" />
                </label>
                <label class="field">
                  <span>服务名</span>
                  <input v-model="tftpConfig.service_name" />
                </label>
                <label class="field">
                  <span>运行用户</span>
                  <input v-model="tftpConfig.username" />
                </label>
                <label class="field">
                  <span>监听地址</span>
                  <input v-model="tftpConfig.address" />
                </label>
                <label class="field">
                  <span>启动选项</span>
                  <input v-model="tftpConfig.options" />
                </label>
                <label class="checkbox-field">
                  <input v-model="tftpConfig.manage_config" type="checkbox" />
                  <span>允许服务端管理配置文件与重启 service</span>
                </label>
                <label class="checkbox-field">
                  <input v-model="tftpConfig.reconcile_on_start" type="checkbox" />
                  <span>启动时自动 reconcile</span>
                </label>
              </template>
            </div>
          </section>

          <section class="panel nested-panel">
            <div class="panel-heading compact">
              <h4>运行状态</h4>
            </div>
            <dl class="key-value-list">
              <div>
                <dt>健康状态</dt>
                <dd>{{ tftpStatus.healthy ? "正常" : "异常" }}</dd>
              </div>
              <div>
                <dt>目录可写</dt>
                <dd>{{ tftpStatus.writable ? "可写" : "不可写" }}</dd>
              </div>
              <div>
                <dt>根目录</dt>
                <dd>{{ tftpStatus.root_dir }}</dd>
              </div>
              <div>
                <dt>绑定/监听</dt>
                <dd>{{ tftpStatus.bind_addr_or_address || "-" }}</dd>
              </div>
              <div>
                <dt>服务状态</dt>
                <dd>{{ tftpStatus.service_state || "-" }}</dd>
              </div>
              <div>
                <dt>当前计算出的 server_ip</dt>
                <dd>{{ tftpStatus.resolved_server_ip || "-" }}</dd>
              </div>
              <div>
                <dt>当前计算出的 netmask</dt>
                <dd>{{ tftpStatus.resolved_netmask || "-" }}</dd>
              </div>
            </dl>
            <p v-if="tftpStatus.last_error" class="diagnostic-error">
              {{ tftpStatus.last_error }}
            </p>
          </section>
        </div>
      </template>
    </div>
  </section>
</template>

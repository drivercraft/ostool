<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import StatusPill from "@/components/StatusPill.vue";
import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { TftpConfig, TftpStatus } from "@/types/api";
import { describeTftpStatus } from "@/utils/tftpStatus";

const ui = useUiStore();
const loading = ref(true);
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
          <h3>TFTP 配置与状态</h3>
        </div>
        <div class="toolbar-actions">
          <button class="btn btn-ghost" @click="loadTftp">刷新</button>
          <button class="btn btn-ghost" :disabled="reconciling" @click="reconcile">
            {{ reconciling ? "执行中..." : "执行 Reconcile" }}
          </button>
        </div>
      </div>

      <div v-if="loading" class="empty-state">正在加载 TFTP 配置...</div>
      <template v-else-if="tftpConfig && tftpStatus">
        <div class="split-grid">
          <section class="panel nested-panel">
            <div class="panel-heading compact">
              <h4>启动配置</h4>
              <StatusPill :tone="tone.tone" :label="tone.label" />
            </div>
            <dl class="key-value-list">
              <div>
                <dt>Provider</dt>
                <dd>{{ tftpConfig.provider }}</dd>
              </div>
              <div>
                <dt>启用状态</dt>
                <dd>{{ tftpConfig.enabled ? "已启用" : "已关闭" }}</dd>
              </div>
              <div>
                <dt>根目录</dt>
                <dd>{{ tftpConfig.root_dir }}</dd>
              </div>
              <template v-if="tftpConfig.provider === 'builtin'">
                <div>
                  <dt>绑定地址</dt>
                  <dd>{{ tftpConfig.bind_addr }}</dd>
                </div>
              </template>
              <template v-else>
                <div>
                  <dt>配置文件</dt>
                  <dd>{{ tftpConfig.config_path }}</dd>
                </div>
                <div>
                  <dt>服务名</dt>
                  <dd>{{ tftpConfig.service_name }}</dd>
                </div>
                <div>
                  <dt>运行用户</dt>
                  <dd>{{ tftpConfig.username || "-" }}</dd>
                </div>
                <div>
                  <dt>监听地址</dt>
                  <dd>{{ tftpConfig.address }}</dd>
                </div>
                <div>
                  <dt>启动选项</dt>
                  <dd>{{ tftpConfig.options }}</dd>
                </div>
                <div>
                  <dt>管理系统配置</dt>
                  <dd>{{ tftpConfig.manage_config ? "允许" : "不允许" }}</dd>
                </div>
                <div>
                  <dt>启动时 Reconcile</dt>
                  <dd>{{ tftpConfig.reconcile_on_start ? "启用" : "关闭" }}</dd>
                </div>
              </template>
            </dl>
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

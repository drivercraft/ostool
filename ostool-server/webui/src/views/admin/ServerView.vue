<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { AdminServerConfigResponse, NetworkInterfaceSummary } from "@/types/api";

type SettingsTabKey = "site" | "policy" | "runtime" | "readonly";

const settingsTabs: Array<{ key: SettingsTabKey; label: string }> = [
  { key: "site", label: "站点信息" },
  { key: "policy", label: "账号与租赁" },
  { key: "runtime", label: "网络与上传" },
  { key: "readonly", label: "只读信息" },
];

const ui = useUiStore();
const loading = ref(true);
const saving = ref(false);
const activeTab = ref<SettingsTabKey>("site");
const config = ref<AdminServerConfigResponse | null>(null);
const initialConfig = ref<AdminServerConfigResponse | null>(null);
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

function cloneConfig(value: AdminServerConfigResponse): AdminServerConfigResponse {
  return JSON.parse(JSON.stringify(value)) as AdminServerConfigResponse;
}

async function loadConfig() {
  loading.value = true;
  try {
    const [serverConfig, interfaces] = await Promise.all([
      api.admin.getServerConfig(),
      api.admin.listNetworkInterfaces(),
    ]);
    config.value = serverConfig;
    initialConfig.value = cloneConfig(serverConfig);
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
  config.value.site.default_lease_minutes = Math.trunc(Number(config.value.site.default_lease_minutes));
  config.value.site.max_lease_minutes = Math.trunc(Number(config.value.site.max_lease_minutes));
  if (config.value.site.default_lease_minutes < 1) {
    ui.setError("默认租赁时长必须大于 0 分钟");
    return;
  }
  if (config.value.site.max_lease_minutes < config.value.site.default_lease_minutes) {
    ui.setError("最大租赁时长不能小于默认租赁时长");
    return;
  }

  saving.value = true;
  try {
    config.value = await api.admin.updateServerConfig({
      editable: config.value.editable,
      site: config.value.site,
    });
    initialConfig.value = cloneConfig(config.value);
    ui.setSuccess("已保存系统设置");
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    saving.value = false;
  }
}

function resetCurrentTab() {
  if (!config.value || !initialConfig.value || activeTab.value === "readonly") {
    return;
  }

  if (activeTab.value === "site") {
    config.value.site.site_name = initialConfig.value.site.site_name;
    config.value.site.site_subtitle = initialConfig.value.site.site_subtitle;
    config.value.site.logo_url = initialConfig.value.site.logo_url;
    config.value.site.favicon_url = initialConfig.value.site.favicon_url;
    config.value.site.announcement = initialConfig.value.site.announcement;
    config.value.site.maintenance_mode = initialConfig.value.site.maintenance_mode;
    return;
  }

  if (activeTab.value === "policy") {
    config.value.site.registration_mode = initialConfig.value.site.registration_mode;
    config.value.site.self_service_enabled = initialConfig.value.site.self_service_enabled;
    config.value.site.default_lease_minutes = initialConfig.value.site.default_lease_minutes;
    config.value.site.max_lease_minutes = initialConfig.value.site.max_lease_minutes;
    config.value.site.support_email = initialConfig.value.site.support_email;
    config.value.site.support_url = initialConfig.value.site.support_url;
    return;
  }

  config.value.editable.network = { ...initialConfig.value.editable.network };
  config.value.editable.upload_limits = { ...initialConfig.value.editable.upload_limits };
}

async function refreshNetworkInterfaces() {
  try {
    networkInterfaces.value = await api.admin.listNetworkInterfaces();
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
  <section class="page-grid settings-page">
    <div class="panel">
      <div v-if="loading" class="empty-state">正在加载 server 配置...</div>
      <template v-else-if="config">
        <div class="tab-list settings-tab-list" role="tablist" aria-label="系统设置标签">
          <button
            v-for="tab in settingsTabs"
            :id="`settings-tab-${tab.key}`"
            :key="tab.key"
            class="tab-button"
            :class="{ 'is-active': activeTab === tab.key }"
            type="button"
            role="tab"
            :aria-selected="activeTab === tab.key"
            :aria-controls="`settings-panel-${tab.key}`"
            @click="activeTab = tab.key"
          >
            <span class="tab-title">{{ tab.label }}</span>
          </button>
        </div>

        <div class="settings-scroll">
          <section
            id="settings-panel-site"
            class="tab-panel settings-tab-panel"
            role="tabpanel"
            aria-labelledby="settings-tab-site"
            :hidden="activeTab !== 'site'"
          >
            <div class="settings-card">
              <div class="settings-card-head">
                <div class="settings-card-title">站点信息</div>
              </div>
              <div class="form-grid two-columns">
                <label class="field">
                  <span>站点名称</span>
                  <input v-model="config.site.site_name" autocomplete="off" />
                </label>
                <label class="field">
                  <span>站点副标题</span>
                  <input v-model="config.site.site_subtitle" autocomplete="off" />
                </label>
                <label class="field">
                  <span>Logo URL</span>
                  <input v-model="config.site.logo_url" placeholder="可选" autocomplete="off" />
                </label>
                <label class="field">
                  <span>Favicon URL</span>
                  <input v-model="config.site.favicon_url" placeholder="可选" autocomplete="off" />
                </label>
                <label class="field form-grid-wide">
                  <span>平台公告</span>
                  <textarea v-model="config.site.announcement" rows="3" placeholder="可选" />
                </label>
                <label class="check-row form-grid-wide">
                  <input v-model="config.site.maintenance_mode" type="checkbox" />
                  <span>维护模式</span>
                </label>
              </div>
            </div>
          </section>

          <section
            id="settings-panel-policy"
            class="tab-panel settings-tab-panel"
            role="tabpanel"
            aria-labelledby="settings-tab-policy"
            :hidden="activeTab !== 'policy'"
          >
            <div class="settings-card">
              <div class="settings-card-head">
                <div class="settings-card-title">账号与租赁策略</div>
              </div>
              <div class="form-grid two-columns">
                <label class="field">
                  <span>自助注册策略</span>
                  <select v-model="config.site.registration_mode">
                    <option value="closed">关闭注册（仅管理员开通）</option>
                    <option value="auto">自动生效</option>
                    <option value="approval">管理员审核</option>
                  </select>
                </label>
                <label class="check-row">
                  <input v-model="config.site.self_service_enabled" type="checkbox" />
                  <span>允许普通用户自助租赁</span>
                </label>
                <label class="field">
                  <span>默认租赁时长（分钟）</span>
                  <input v-model.number="config.site.default_lease_minutes" type="number" min="1" step="1" />
                </label>
                <label class="field">
                  <span>最大租赁时长（分钟）</span>
                  <input v-model.number="config.site.max_lease_minutes" type="number" min="1" step="1" />
                </label>
                <label class="field">
                  <span>支持邮箱</span>
                  <input v-model="config.site.support_email" placeholder="可选" autocomplete="off" />
                </label>
                <label class="field">
                  <span>支持链接</span>
                  <input v-model="config.site.support_url" placeholder="可选" autocomplete="off" />
                </label>
              </div>
            </div>
          </section>

          <section
            id="settings-panel-runtime"
            class="tab-panel settings-tab-panel"
            role="tabpanel"
            aria-labelledby="settings-tab-runtime"
            :hidden="activeTab !== 'runtime'"
          >
            <div class="settings-card">
              <div class="settings-card-head">
                <div class="settings-card-title">网络与上传</div>
              </div>
              <div class="form-grid two-columns">
                <label class="field form-grid-wide">
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
                    <button class="btn btn-ghost btn-sm" type="button" @click="refreshNetworkInterfaces">
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
            </div>
          </section>

          <section
            id="settings-panel-readonly"
            class="tab-panel settings-tab-panel"
            role="tabpanel"
            aria-labelledby="settings-tab-readonly"
            :hidden="activeTab !== 'readonly'"
          >
            <div class="settings-card">
              <div class="settings-card-head">
                <div class="settings-card-title">只读信息</div>
              </div>
              <dl class="key-value-list settings-readonly-list">
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
            </div>
          </section>
        </div>

        <div class="settings-footer">
          <button
            class="btn btn-ghost"
            type="button"
            :disabled="saving || activeTab === 'readonly'"
            @click="resetCurrentTab"
          >
            恢复默认
          </button>
          <button class="btn btn-primary" type="button" :disabled="saving || !config" @click="saveConfig">
            {{ saving ? "保存中..." : "保存配置" }}
          </button>
        </div>
      </template>
    </div>
  </section>
</template>

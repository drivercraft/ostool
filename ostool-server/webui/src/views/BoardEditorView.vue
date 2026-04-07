<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";

import { api } from "@/api/client";
import { useUiStore } from "@/stores/ui";
import type {
  AdminBoardUpsertRequest,
  BoardConfig,
  BootConfig,
  DtbFileResponse,
  PowerManagementConfig,
  SerialPortKeyKind,
  SerialPortSummary,
} from "@/types/api";

type PowerManagementKind = "custom" | "zhongsheng_relay";
type BootKind = "uboot" | "pxe";

interface BoardEditorFormState {
  id: string;
  board_type: string;
  tags_text: string;
  notes: string;
  disabled: boolean;
  serial_enabled: boolean;
  serial_key_kind: SerialPortKeyKind;
  serial_key_value: string;
  serial_baud_rate: number;
  power_management_kind: PowerManagementKind;
  power_on_cmd: string;
  power_off_cmd: string;
  relay_serial_key_kind: SerialPortKeyKind;
  relay_serial_key_value: string;
  boot_kind: BootKind;
  use_tftp: boolean;
  dtb_name: string;
  pxe_notes: string;
}

const DEFAULT_SERIAL_BAUD_RATE = 115_200;

const route = useRoute();
const router = useRouter();
const ui = useUiStore();

const loading = ref(true);
const saving = ref(false);
const deleting = ref(false);
const refreshingSerials = ref(false);
const uploadingDtb = ref(false);
const validationError = ref("");
const form = ref<BoardEditorFormState>(defaultFormState());
const serialPorts = ref<SerialPortSummary[]>([]);
const dtbs = ref<DtbFileResponse[]>([]);
const dtbUploadName = ref("");
const dtbUploadFile = ref<File | null>(null);
const dtbFileInput = ref<HTMLInputElement | null>(null);
const showingDtbUploadModal = ref(false);
const isEditing = computed(() => typeof route.params.boardId === "string");
const boardId = computed(() => route.params.boardId as string | undefined);
const selectedBoardSerialSummary = computed(() => {
  const selected = selectedBoardSerialOptionValue();
  return serialPorts.value.find((port) => serialOptionValue(port) === selected) ?? null;
});
const selectedRelaySerialSummary = computed(() => {
  const selected = selectedRelaySerialOptionValue();
  return serialPorts.value.find((port) => serialOptionValue(port) === selected) ?? null;
});

function defaultFormState(): BoardEditorFormState {
  return {
    id: "",
    board_type: "",
    tags_text: "",
    notes: "",
    disabled: false,
    serial_enabled: false,
    serial_key_kind: "serial_number",
    serial_key_value: "",
    serial_baud_rate: DEFAULT_SERIAL_BAUD_RATE,
    power_management_kind: "custom",
    power_on_cmd: "",
    power_off_cmd: "",
    relay_serial_key_kind: "serial_number",
    relay_serial_key_value: "",
    boot_kind: "uboot",
    use_tftp: false,
    dtb_name: "",
    pxe_notes: "",
  };
}

function boardToFormState(board: BoardConfig): BoardEditorFormState {
  const next = defaultFormState();
  next.id = board.id;
  next.board_type = board.board_type;
  next.tags_text = board.tags.join(", ");
  next.notes = board.notes ?? "";
  next.disabled = board.disabled;

  if (board.serial) {
    next.serial_enabled = true;
    next.serial_key_kind = board.serial.key.kind;
    next.serial_key_value = board.serial.key.value;
    next.serial_baud_rate = board.serial.baud_rate;
  }

  if (board.power_management.kind === "custom") {
    next.power_management_kind = "custom";
    next.power_on_cmd = board.power_management.power_on_cmd;
    next.power_off_cmd = board.power_management.power_off_cmd;
  } else {
    next.power_management_kind = "zhongsheng_relay";
    next.relay_serial_key_kind = board.power_management.key.kind;
    next.relay_serial_key_value = board.power_management.key.value;
  }

  if (board.boot.kind === "uboot") {
    next.boot_kind = "uboot";
    next.use_tftp = board.boot.use_tftp;
    next.dtb_name = board.boot.dtb_name ?? "";
  } else {
    next.boot_kind = "pxe";
    next.pxe_notes = board.boot.notes ?? "";
  }

  return next;
}

function trimToNull(value: string): string | null {
  const trimmed = value.trim();
  return trimmed ? trimmed : null;
}

function splitTags(tagsText: string): string[] {
  return tagsText
    .split(/[,\n]/)
    .map((tag) => tag.trim())
    .filter((tag) => tag.length > 0);
}

function buildBootConfig(): BootConfig {
  if (form.value.boot_kind === "uboot") {
    return {
      kind: "uboot",
      use_tftp: form.value.use_tftp,
      dtb_name: trimToNull(form.value.dtb_name),
    };
  }

  return {
    kind: "pxe",
    notes: trimToNull(form.value.pxe_notes),
  };
}

function buildPowerManagementConfig(): PowerManagementConfig {
  if (form.value.power_management_kind === "custom") {
    return {
      kind: "custom",
      power_on_cmd: form.value.power_on_cmd.trim(),
      power_off_cmd: form.value.power_off_cmd.trim(),
    };
  }

  return {
    kind: "zhongsheng_relay",
    key: {
      kind: form.value.relay_serial_key_kind,
      value: form.value.relay_serial_key_value.trim(),
    },
  };
}

function buildRequestPayload(): AdminBoardUpsertRequest {
  return {
    id: trimToNull(form.value.id),
    board_type: form.value.board_type.trim(),
    tags: splitTags(form.value.tags_text),
    notes: trimToNull(form.value.notes),
    disabled: form.value.disabled,
    serial: form.value.serial_enabled
      ? {
          key: {
            kind: form.value.serial_key_kind,
            value: form.value.serial_key_value.trim(),
          },
          baud_rate: form.value.serial_baud_rate,
        }
      : null,
    power_management: buildPowerManagementConfig(),
    boot: buildBootConfig(),
  };
}

function validateForm(): string {
  const errors: string[] = [];

  if (!form.value.board_type.trim()) {
    errors.push("board_type 不能为空");
  }
  if (form.value.id.includes("/") || form.value.id.includes("\\")) {
    errors.push("板子 ID 不能包含路径分隔符");
  }
  if (form.value.serial_enabled && !form.value.serial_key_value.trim()) {
    errors.push("启用串口时必须选择串口设备");
  }
  if (form.value.serial_enabled && (!Number.isFinite(form.value.serial_baud_rate) || form.value.serial_baud_rate <= 0)) {
    errors.push("启用串口时波特率必须大于 0");
  }
  if (form.value.power_management_kind === "custom") {
    if (!form.value.power_on_cmd.trim()) {
      errors.push("Custom 电源管理必须填写开机命令");
    }
    if (!form.value.power_off_cmd.trim()) {
      errors.push("Custom 电源管理必须填写关机命令");
    }
  }
  if (form.value.power_management_kind === "zhongsheng_relay" && !form.value.relay_serial_key_value.trim()) {
    errors.push("中盛继电模块必须选择串口设备");
  }
  return errors.join("\n");
}

function serialOptionValue(port: SerialPortSummary) {
  if (port.primary_key_kind && port.primary_key_value) {
    return `${port.primary_key_kind}:${port.primary_key_value}`;
  }
  return `unstable:${port.current_device_path}`;
}

function serialOptionLabel(port: SerialPortSummary) {
  const primary = port.primary_key_kind && port.primary_key_value
    ? `[${port.primary_key_kind === "serial_number" ? "SN" : "USB PATH"}] ${port.primary_key_value}`
    : `[UNSTABLE] ${port.current_device_path}`;
  const details = [
    port.usb_path,
    port.current_device_path,
    port.manufacturer,
    port.product,
  ].filter((value): value is string => Boolean(value));
  return details.length === 0 ? primary : `${primary} | ${details.join(" / ")}`;
}

function selectedBoardSerialOptionValue() {
  const value = form.value.serial_key_value.trim();
  if (!value) {
    return "";
  }
  return `${form.value.serial_key_kind}:${value}`;
}

function selectedRelaySerialOptionValue() {
  const value = form.value.relay_serial_key_value.trim();
  if (!value) {
    return "";
  }
  return `${form.value.relay_serial_key_kind}:${value}`;
}

function parseSerialSelection(value: string): { kind: SerialPortKeyKind; value: string } | null {
  const [kind, ...rest] = value.split(":");
  if (kind === "serial_number" || kind === "usb_path") {
    return {
      kind,
      value: rest.join(":"),
    };
  }
  return null;
}

function parseBoardSerialSelection(value: string) {
  const parsed = parseSerialSelection(value);
  if (parsed) {
    form.value.serial_key_kind = parsed.kind;
    form.value.serial_key_value = parsed.value;
  }
}

function parseRelaySerialSelection(value: string) {
  const parsed = parseSerialSelection(value);
  if (parsed) {
    form.value.relay_serial_key_kind = parsed.kind;
    form.value.relay_serial_key_value = parsed.value;
  }
}

function boardSerialOptions(currentValue: string) {
  const options = new Map<string, { label: string; disabled: boolean }>();
  for (const port of serialPorts.value) {
    const value = serialOptionValue(port);
    options.set(value, {
      label: serialOptionLabel(port),
      disabled: !port.stable_identity,
    });
  }
  const trimmed = currentValue.trim();
  if (trimmed && !options.has(trimmed)) {
    const keyKind = form.value.serial_key_kind === "serial_number" ? "SN" : "USB PATH";
    options.set(trimmed, {
      label: `[${keyKind}] ${form.value.serial_key_value} (当前配置，未检测到)`,
      disabled: false,
    });
  }
  return Array.from(options.entries()).map(([value, option]) => ({
    value,
    label: option.label,
    disabled: option.disabled,
  }));
}

function relaySerialOptions(currentValue: string) {
  const options = new Map<string, { label: string; disabled: boolean }>();
  for (const port of serialPorts.value) {
    if (!port.stable_identity) {
      continue;
    }
    const value = serialOptionValue(port);
    options.set(value, {
      label: serialOptionLabel(port),
      disabled: false,
    });
  }
  const trimmed = currentValue.trim();
  if (trimmed && !options.has(trimmed)) {
    const keyKind = form.value.relay_serial_key_kind === "serial_number" ? "SN" : "USB PATH";
    options.set(trimmed, {
      label: `[${keyKind}] ${form.value.relay_serial_key_value} (当前配置，未检测到)`,
      disabled: false,
    });
  }
  return Array.from(options.entries()).map(([value, option]) => ({
    value,
    label: option.label,
    disabled: option.disabled,
  }));
}

function serialPrimaryLabel(kind: SerialPortKeyKind) {
  return kind === "serial_number" ? "SN" : "USB PATH";
}

function selectedBoardSerialDescription() {
  if (selectedBoardSerialSummary.value) {
    const port = selectedBoardSerialSummary.value;
    const secondary = [
      port.usb_path,
      port.current_device_path,
      port.manufacturer,
      port.product,
      port.usb_vendor_id !== null && port.usb_product_id !== null
        ? `VID:PID ${port.usb_vendor_id.toString(16).padStart(4, "0")}:${port.usb_product_id
            .toString(16)
            .padStart(4, "0")}`
        : null,
    ].filter((value): value is string => Boolean(value));
    return {
      primaryLabel: serialPrimaryLabel(port.primary_key_kind!),
      primaryValue: port.primary_key_value!,
      secondary,
      unresolved: false,
    };
  }

  if (!form.value.serial_key_value.trim()) {
    return null;
  }

  return {
    primaryLabel: serialPrimaryLabel(form.value.serial_key_kind),
    primaryValue: form.value.serial_key_value.trim(),
    secondary: ["当前未检测到对应设备"],
    unresolved: true,
  };
}

function selectedRelaySerialDescription() {
  if (selectedRelaySerialSummary.value) {
    const port = selectedRelaySerialSummary.value;
    const secondary = [
      port.usb_path,
      port.current_device_path,
      port.manufacturer,
      port.product,
      port.usb_vendor_id !== null && port.usb_product_id !== null
        ? `VID:PID ${port.usb_vendor_id.toString(16).padStart(4, "0")}:${port.usb_product_id
            .toString(16)
            .padStart(4, "0")}`
        : null,
    ].filter((value): value is string => Boolean(value));
    return {
      primaryLabel: serialPrimaryLabel(port.primary_key_kind!),
      primaryValue: port.primary_key_value!,
      secondary,
      unresolved: false,
    };
  }

  if (!form.value.relay_serial_key_value.trim()) {
    return null;
  }

  return {
    primaryLabel: serialPrimaryLabel(form.value.relay_serial_key_kind),
    primaryValue: form.value.relay_serial_key_value.trim(),
    secondary: ["当前未检测到对应设备"],
    unresolved: true,
  };
}

function dtbOptions(currentValue: string) {
  const options = new Map<string, string>();
  for (const dtb of dtbs.value) {
    options.set(dtb.name, `${dtb.name} (${dtb.relative_tftp_path_template})`);
  }
  const trimmed = currentValue.trim();
  if (trimmed && !options.has(trimmed)) {
    options.set(trimmed, `${trimmed} (当前配置，未检测到)`);
  }
  return Array.from(options.entries()).map(([value, label]) => ({ value, label }));
}

async function loadSerialPorts() {
  serialPorts.value = await api.listSerialPorts();
}

async function refreshSerialPorts() {
  refreshingSerials.value = true;
  try {
    await loadSerialPorts();
    ui.setSuccess("已刷新串口列表");
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    refreshingSerials.value = false;
  }
}

async function loadEditor() {
  loading.value = true;
  validationError.value = "";
  ui.clearMessages();

  try {
    const [ports, dtbList, board] = await Promise.all([
      api.listSerialPorts(),
      api.listDtbs(),
      isEditing.value && boardId.value ? api.getBoard(boardId.value) : Promise.resolve(null),
    ]);
    serialPorts.value = ports;
    dtbs.value = dtbList;
    form.value = board ? boardToFormState(board) : defaultFormState();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

function onDtbFileChange(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0] ?? null;
  dtbUploadFile.value = file;
  if (file) {
    dtbUploadName.value = file.name;
  }
}

function openDtbUploadModal() {
  showingDtbUploadModal.value = true;
}

function closeDtbUploadModal() {
  showingDtbUploadModal.value = false;
  dtbUploadName.value = "";
  dtbUploadFile.value = null;
  if (dtbFileInput.value) {
    dtbFileInput.value.value = "";
  }
}

async function uploadDtbAndSelect() {
  if (!dtbUploadFile.value) {
    ui.setError("请选择要上传的 DTB 文件");
    return;
  }
  const dtbName = dtbUploadName.value.trim() || dtbUploadFile.value.name;
  if (!dtbName) {
    ui.setError("请填写 DTB 文件名");
    return;
  }

  uploadingDtb.value = true;
  try {
    const created = await api.createDtb(dtbName, dtbUploadFile.value);
    dtbs.value = [...dtbs.value.filter((item) => item.name !== created.name), created].sort((a, b) =>
      a.name.localeCompare(b.name),
    );
    form.value.dtb_name = created.name;
    closeDtbUploadModal();
    ui.setSuccess(`已上传 DTB ${created.name}`);
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    uploadingDtb.value = false;
  }
}

async function saveBoard() {
  validationError.value = validateForm();
  if (validationError.value) {
    return;
  }

  saving.value = true;
  try {
    const payload = buildRequestPayload();
    const saved = isEditing.value && boardId.value
      ? await api.updateBoard(boardId.value, payload)
      : await api.createBoard(payload);
    form.value = boardToFormState(saved);
    ui.setSuccess(`已保存开发板 ${saved.id}`);
    await router.push(`/boards/${saved.id}`);
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
          <button class="ghost-button" @click="loadEditor">刷新表单</button>
          <button class="ghost-button" :disabled="refreshingSerials" @click="refreshSerialPorts">
            {{ refreshingSerials ? "刷新串口中..." : "刷新串口" }}
          </button>
          <button class="primary-button" :disabled="saving || loading" @click="saveBoard">
            {{ saving ? "保存中..." : "保存配置" }}
          </button>
        </div>
      </div>

      <div v-if="loading" class="empty-state">
        <div class="empty-state-icon">&#9641;</div>
        正在加载开发板配置...
      </div>
      <template v-else>
        <p v-if="validationError" class="diagnostic-error">{{ validationError }}</p>

        <!-- 基本信息 -->
        <section class="form-section">
          <div class="form-section-header">
            <span class="form-section-icon info">&#9776;</span>
            <h4>基本信息</h4>
          </div>
          <div class="form-grid two-columns">
            <label class="field">
              <span>板型</span>
              <input v-model="form.board_type" placeholder="例如 rk3568" />
            </label>
            <label class="field">
              <span>板子 ID</span>
              <input v-model="form.id" placeholder="留空则自动分配 {board type}-{num}" />
              <small class="field-hint">
                编辑已有开发板时留空会保留当前 ID。
              </small>
            </label>
          </div>

          <div class="form-grid two-columns" style="margin-top: 16px">
            <label class="field">
              <span>标签</span>
              <input v-model="form.tags_text" placeholder="lab, usb" />
            </label>
            <label class="toggle-field">
              <span class="toggle-switch">
                <input v-model="form.disabled" type="checkbox" />
                <span class="toggle-track" />
                <span class="toggle-knob" />
              </span>
              <span class="toggle-label">禁用该开发板</span>
            </label>
          </div>

          <label class="field" style="margin-top: 16px">
            <span>备注</span>
            <textarea v-model="form.notes" rows="4" />
          </label>
        </section>

        <!-- 串口配置 -->
        <section class="form-section">
          <div class="form-section-header">
            <span class="form-section-icon serial">&#8982;</span>
            <h4>串口配置</h4>
          </div>
          <label class="toggle-field">
            <span class="toggle-switch">
              <input v-model="form.serial_enabled" type="checkbox" />
              <span class="toggle-track" />
              <span class="toggle-knob" />
            </span>
            <span class="toggle-label">启用串口</span>
          </label>

          <div v-if="form.serial_enabled" class="form-grid two-columns" style="margin-top: 18px">
            <label class="field">
              <span>串口设备</span>
              <select
                :value="selectedBoardSerialOptionValue()"
                @change="parseBoardSerialSelection(($event.target as HTMLSelectElement).value)"
              >
                <option value="">请选择串口设备</option>
                <option
                  v-for="option in boardSerialOptions(selectedBoardSerialOptionValue())"
                  :key="option.value"
                  :value="option.value"
                  :disabled="option.disabled"
                >
                  {{ option.label }}
                </option>
              </select>
              <div v-if="selectedBoardSerialDescription()" class="serial-key-card">
                <span class="serial-key-badge">{{ selectedBoardSerialDescription()!.primaryLabel }}</span>
                <strong>{{ selectedBoardSerialDescription()!.primaryValue }}</strong>
                <p
                  v-for="detail in selectedBoardSerialDescription()!.secondary"
                  :key="detail"
                  class="serial-key-secondary"
                  :class="{ unresolved: selectedBoardSerialDescription()!.unresolved }"
                >
                  {{ detail }}
                </p>
              </div>
            </label>
            <label class="field">
              <span>波特率</span>
              <input v-model.number="form.serial_baud_rate" type="number" min="1" />
            </label>
          </div>
        </section>

        <!-- 电源管理 -->
        <section class="form-section">
          <div class="form-section-header">
            <span class="form-section-icon power">&#9889;</span>
            <h4>电源管理</h4>
          </div>
          <label class="field">
            <span>电源管理类型</span>
            <select v-model="form.power_management_kind">
              <option value="custom">Custom</option>
              <option value="zhongsheng_relay">中盛继电模块</option>
            </select>
          </label>

          <div v-if="form.power_management_kind === 'custom'" class="form-grid two-columns" style="margin-top: 16px">
            <label class="field">
              <span>开机命令</span>
              <input v-model="form.power_on_cmd" />
            </label>
            <label class="field">
              <span>关机命令</span>
              <input v-model="form.power_off_cmd" />
            </label>
          </div>

          <label v-else class="field" style="margin-top: 16px">
            <span>继电模块串口</span>
            <select
              :value="selectedRelaySerialOptionValue()"
              @change="parseRelaySerialSelection(($event.target as HTMLSelectElement).value)"
            >
              <option value="">请选择串口设备</option>
              <option
                v-for="option in relaySerialOptions(selectedRelaySerialOptionValue())"
                :key="option.value"
                :value="option.value"
                :disabled="option.disabled"
              >
                {{ option.label }}
              </option>
            </select>
            <div v-if="selectedRelaySerialDescription()" class="serial-key-card">
              <span class="serial-key-badge neutral">{{ selectedRelaySerialDescription()!.primaryLabel }}</span>
              <strong>{{ selectedRelaySerialDescription()!.primaryValue }}</strong>
              <p
                v-for="detail in selectedRelaySerialDescription()!.secondary"
                :key="detail"
                class="serial-key-secondary"
                :class="{ unresolved: selectedRelaySerialDescription()!.unresolved }"
              >
                {{ detail }}
              </p>
            </div>
          </label>
        </section>

        <!-- 启动方式 -->
        <section class="form-section">
          <div class="form-section-header">
            <span class="form-section-icon boot">&#9654;</span>
            <h4>启动方式</h4>
          </div>
          <label class="field">
            <span>启动模式</span>
            <select v-model="form.boot_kind">
              <option value="uboot">U-Boot</option>
              <option value="pxe">PXE</option>
            </select>
          </label>

          <template v-if="form.boot_kind === 'uboot'">
            <label class="toggle-field" style="margin-top: 16px">
              <span class="toggle-switch">
                <input v-model="form.use_tftp" type="checkbox" />
                <span class="toggle-track" />
                <span class="toggle-knob" />
              </span>
              <span class="toggle-label">使用 TFTP 启动</span>
            </label>

            <div class="split-grid dtb-config-grid" style="margin-top: 18px">
              <section class="panel nested-panel dtb-selection-panel">
                <div class="panel-heading compact">
                  <div>
                    <h4>预设 DTB</h4>
                    <p class="field-hint">为当前开发板选择默认使用的设备树文件。</p>
                  </div>
                </div>
                <label class="field">
                  <span>已选择 DTB</span>
                  <select v-model="form.dtb_name">
                    <option value="">不使用预设 DTB</option>
                    <option
                      v-for="option in dtbOptions(form.dtb_name)"
                      :key="option.value"
                      :value="option.value"
                    >
                      {{ option.label }}
                    </option>
                  </select>
                </label>
                <p class="selection-caption">
                  {{ form.dtb_name ? `当前选择：${form.dtb_name}` : "当前未绑定预设 DTB" }}
                </p>
              </section>

              <section class="panel nested-panel dtb-action-panel">
                <div class="panel-heading compact">
                  <div>
                    <h4>新增 DTB</h4>
                    <p class="field-hint">上传新的 DTB 后会自动加入列表，并直接选中。</p>
                  </div>
                </div>
                <button class="primary-button" type="button" @click="openDtbUploadModal">
                  新增 DTB
                </button>
              </section>
            </div>
          </template>

          <label v-else class="field" style="margin-top: 16px">
            <span>PXE 备注</span>
            <textarea v-model="form.pxe_notes" rows="4" />
          </label>
        </section>

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

  <div
    v-if="showingDtbUploadModal"
    class="modal-overlay"
    @click.self="closeDtbUploadModal"
  >
    <div class="modal-card">
      <div class="panel-heading compact">
        <div>
          <p class="eyebrow">新增 DTB</p>
          <h4>上传并绑定到当前开发板</h4>
        </div>
      </div>

      <div class="form-grid two-columns">
        <label class="field">
          <span>文件名</span>
          <input v-model="dtbUploadName" placeholder="例如 board.dtb" />
        </label>
        <label class="field">
          <span>选择文件</span>
          <input
            ref="dtbFileInput"
            type="file"
            accept=".dtb,application/octet-stream"
            @change="onDtbFileChange"
          />
        </label>
      </div>

      <div class="toolbar-actions modal-actions">
        <button class="ghost-button" type="button" :disabled="uploadingDtb" @click="closeDtbUploadModal">
          取消
        </button>
        <button class="primary-button" type="button" :disabled="uploadingDtb" @click="uploadDtbAndSelect">
          {{ uploadingDtb ? "上传中..." : "上传并选中" }}
        </button>
      </div>
    </div>
  </div>
</template>

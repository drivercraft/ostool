<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";

import { api } from "@/api/client";
import { useUiStore } from "@/stores/ui";
import type {
  AdminBoardUpsertRequest,
  BoardConfig,
  BootConfig,
  PowerManagementConfig,
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
  serial_port: string;
  serial_baud_rate: number;
  power_management_enabled: boolean;
  power_management_kind: PowerManagementKind;
  power_on_cmd: string;
  power_off_cmd: string;
  relay_serial_port: string;
  boot_kind: BootKind;
  use_tftp: boolean;
  kernel_load_addr: string;
  fit_load_addr: string;
  timeout_text: string;
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
const validationError = ref("");
const form = ref<BoardEditorFormState>(defaultFormState());
const serialPorts = ref<SerialPortSummary[]>([]);
const isEditing = computed(() => typeof route.params.boardId === "string");
const boardId = computed(() => route.params.boardId as string | undefined);

function defaultFormState(): BoardEditorFormState {
  return {
    id: "",
    board_type: "",
    tags_text: "",
    notes: "",
    disabled: false,
    serial_enabled: false,
    serial_port: "",
    serial_baud_rate: DEFAULT_SERIAL_BAUD_RATE,
    power_management_enabled: false,
    power_management_kind: "custom",
    power_on_cmd: "",
    power_off_cmd: "",
    relay_serial_port: "",
    boot_kind: "uboot",
    use_tftp: false,
    kernel_load_addr: "",
    fit_load_addr: "",
    timeout_text: "",
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
    next.serial_port = board.serial.port;
    next.serial_baud_rate = board.serial.baud_rate;
  }

  if (board.power_management) {
    next.power_management_enabled = true;
    if (board.power_management.kind === "custom") {
      next.power_management_kind = "custom";
      next.power_on_cmd = board.power_management.power_on_cmd;
      next.power_off_cmd = board.power_management.power_off_cmd;
    } else {
      next.power_management_kind = "zhongsheng_relay";
      next.relay_serial_port = board.power_management.serial_port;
    }
  }

  if (board.boot.kind === "uboot") {
    next.boot_kind = "uboot";
    next.use_tftp = board.boot.use_tftp;
    next.kernel_load_addr = board.boot.kernel_load_addr ?? "";
    next.fit_load_addr = board.boot.fit_load_addr ?? "";
    next.timeout_text = board.boot.timeout === null ? "" : String(board.boot.timeout);
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
      kernel_load_addr: trimToNull(form.value.kernel_load_addr),
      fit_load_addr: trimToNull(form.value.fit_load_addr),
      timeout: trimToNull(form.value.timeout_text) === null
        ? null
        : Number.parseInt(form.value.timeout_text, 10),
    };
  }

  return {
    kind: "pxe",
    notes: trimToNull(form.value.pxe_notes),
  };
}

function buildPowerManagementConfig(): PowerManagementConfig | null {
  if (!form.value.power_management_enabled) {
    return null;
  }

  if (form.value.power_management_kind === "custom") {
    return {
      kind: "custom",
      power_on_cmd: form.value.power_on_cmd.trim(),
      power_off_cmd: form.value.power_off_cmd.trim(),
    };
  }

  return {
    kind: "zhongsheng_relay",
    serial_port: form.value.relay_serial_port.trim(),
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
          port: form.value.serial_port.trim(),
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
  if (form.value.serial_enabled && !form.value.serial_port.trim()) {
    errors.push("启用串口时必须选择串口设备");
  }
  if (form.value.serial_enabled && (!Number.isFinite(form.value.serial_baud_rate) || form.value.serial_baud_rate <= 0)) {
    errors.push("启用串口时波特率必须大于 0");
  }
  if (form.value.power_management_enabled && form.value.power_management_kind === "custom") {
    if (!form.value.power_on_cmd.trim()) {
      errors.push("启用 Custom 电源管理时必须填写开机命令");
    }
    if (!form.value.power_off_cmd.trim()) {
      errors.push("启用 Custom 电源管理时必须填写关机命令");
    }
  }
  if (
    form.value.power_management_enabled &&
    form.value.power_management_kind === "zhongsheng_relay" &&
    !form.value.relay_serial_port.trim()
  ) {
    errors.push("启用中盛继电模块时必须选择串口设备");
  }
  if (form.value.boot_kind === "uboot" && trimToNull(form.value.timeout_text) !== null) {
    const timeout = Number.parseInt(form.value.timeout_text, 10);
    if (!Number.isInteger(timeout) || timeout < 0) {
      errors.push("U-Boot 超时必须为空或非负整数");
    }
  }

  return errors.join("\n");
}

function serialOptions(currentValue: string) {
  const options = new Map<string, string>();
  for (const port of serialPorts.value) {
    options.set(port.port_name, port.label);
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
    const [ports, board] = await Promise.all([
      api.listSerialPorts(),
      isEditing.value && boardId.value ? api.getBoard(boardId.value) : Promise.resolve(null),
    ]);
    serialPorts.value = ports;
    form.value = board ? boardToFormState(board) : defaultFormState();
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
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

      <div v-if="loading" class="empty-state">正在加载开发板配置...</div>
      <template v-else>
        <p v-if="validationError" class="diagnostic-error">{{ validationError }}</p>

        <section class="form-section">
          <h4>基本信息</h4>
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

          <div class="form-grid two-columns">
            <label class="field">
              <span>标签</span>
              <input v-model="form.tags_text" placeholder="lab, usb" />
            </label>
            <label class="checkbox-field">
              <input v-model="form.disabled" type="checkbox" />
              <span>禁用该开发板</span>
            </label>
          </div>

          <label class="field">
            <span>备注</span>
            <textarea v-model="form.notes" rows="4" />
          </label>
        </section>

        <section class="form-section">
          <h4>串口配置</h4>
          <label class="checkbox-field">
            <input v-model="form.serial_enabled" type="checkbox" />
            <span>启用串口</span>
          </label>

          <div v-if="form.serial_enabled" class="form-grid two-columns">
            <label class="field">
              <span>串口设备</span>
              <select v-model="form.serial_port">
                <option value="">请选择串口设备</option>
                <option
                  v-for="option in serialOptions(form.serial_port)"
                  :key="option.value"
                  :value="option.value"
                >
                  {{ option.label }}
                </option>
              </select>
            </label>
            <label class="field">
              <span>波特率</span>
              <input v-model.number="form.serial_baud_rate" type="number" min="1" />
            </label>
          </div>
        </section>

        <section class="form-section">
          <h4>电源管理</h4>
          <label class="checkbox-field">
            <input v-model="form.power_management_enabled" type="checkbox" />
            <span>启用电源管理</span>
          </label>

          <template v-if="form.power_management_enabled">
            <label class="field">
              <span>电源管理类型</span>
              <select v-model="form.power_management_kind">
                <option value="custom">Custom</option>
                <option value="zhongsheng_relay">中盛继电模块</option>
              </select>
            </label>

            <div v-if="form.power_management_kind === 'custom'" class="form-grid two-columns">
              <label class="field">
                <span>开机命令</span>
                <input v-model="form.power_on_cmd" />
              </label>
              <label class="field">
                <span>关机命令</span>
                <input v-model="form.power_off_cmd" />
              </label>
            </div>

            <label v-else class="field">
              <span>继电模块串口</span>
              <select v-model="form.relay_serial_port">
                <option value="">请选择串口设备</option>
                <option
                  v-for="option in serialOptions(form.relay_serial_port)"
                  :key="option.value"
                  :value="option.value"
                >
                  {{ option.label }}
                </option>
              </select>
            </label>
          </template>
        </section>

        <section class="form-section">
          <h4>启动方式</h4>
          <label class="field">
            <span>启动模式</span>
            <select v-model="form.boot_kind">
              <option value="uboot">U-Boot</option>
              <option value="pxe">PXE</option>
            </select>
          </label>

          <template v-if="form.boot_kind === 'uboot'">
            <div class="form-grid two-columns">
              <label class="checkbox-field">
                <input v-model="form.use_tftp" type="checkbox" />
                <span>使用 TFTP 启动</span>
              </label>
              <label class="field">
                <span>超时（秒）</span>
                <input v-model="form.timeout_text" type="number" min="0" placeholder="留空表示无超时" />
              </label>
            </div>

            <div class="form-grid two-columns">
              <label class="field">
                <span>FIT 加载地址</span>
                <input v-model="form.fit_load_addr" />
              </label>
              <label class="field">
                <span>内核加载地址</span>
                <input v-model="form.kernel_load_addr" />
              </label>
            </div>
          </template>

          <label v-else class="field">
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
</template>

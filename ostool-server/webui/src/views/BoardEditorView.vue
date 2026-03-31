<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";

import { api } from "@/api/client";
import { useUiStore } from "@/stores/ui";
import type { BoardConfig, SerialPortSummary } from "@/types/api";
import { boardToForm, createDefaultBoardForm, formToBoard } from "@/utils/boardForm";
import { suggestBoardId } from "@/utils/boardIdentity";

const route = useRoute();
const router = useRouter();
const ui = useUiStore();

const loading = ref(true);
const saving = ref(false);
const deleting = ref(false);
const validationError = ref("");
const form = ref(createDefaultBoardForm());
const knownBoards = ref<BoardConfig[]>([]);
const serialPorts = ref<SerialPortSummary[]>([]);
const idMode = ref<"auto" | "manual">("auto");
const isEditing = computed(() => typeof route.params.boardId === "string");
const boardId = computed(() => route.params.boardId as string | undefined);
const knownBoardTypes = computed(() =>
  Array.from(new Set(knownBoards.value.map((board) => board.board_type)))
    .filter(Boolean)
    .sort(),
);
const suggestedBoardId = computed(() =>
  suggestBoardId(form.value.board_type, knownBoards.value, boardId.value),
);
const serialPortOptions = computed(() => {
  const options = [...serialPorts.value];
  const currentPort = form.value.serialPort.trim();
  if (currentPort && !options.some((item) => item.port_name === currentPort)) {
    options.unshift({
      port_name: currentPort,
      port_type: "configured",
      label: `${currentPort} (当前配置，未检测到)`,
      usb_vendor_id: null,
      usb_product_id: null,
      manufacturer: null,
      product: null,
      serial_number: null,
    });
  }
  return options;
});
function validateForm(): boolean {
  const payload = formToBoard(form.value);
  if (!payload.id) {
    validationError.value = "板子 ID 不能为空。";
    return false;
  }
  if (!payload.name) {
    validationError.value = "板子名称不能为空。";
    return false;
  }
  if (!payload.board_type) {
    validationError.value = "板型不能为空。";
    return false;
  }
  if (payload.serial && !payload.serial.port.trim()) {
    validationError.value = "启用串口时必须填写串口设备。";
    return false;
  }
  validationError.value = "";
  return true;
}

async function loadBoard() {
  loading.value = true;
  ui.clearMessages();
  validationError.value = "";

  try {
    const [boards, ports] = await Promise.all([api.listBoards(), api.listSerialPorts()]);
    knownBoards.value = boards;
    serialPorts.value = ports;

    if (isEditing.value && boardId.value) {
      const board = await api.getBoard(boardId.value);
      form.value = boardToForm(board);
      idMode.value = "manual";
    } else {
      form.value = createDefaultBoardForm();
      idMode.value = "auto";
    }
  } catch (error) {
    ui.setError((error as Error).message);
  } finally {
    loading.value = false;
  }
}

async function refreshSerialPorts() {
  try {
    serialPorts.value = await api.listSerialPorts();
    ui.setSuccess("已刷新可用串口列表");
  } catch (error) {
    ui.setError((error as Error).message);
  }
}

async function saveBoard() {
  if (!validateForm()) {
    return;
  }

  const payload = formToBoard(form.value);
  saving.value = true;
  try {
    const board: BoardConfig = isEditing.value && boardId.value
      ? await api.updateBoard(boardId.value, payload)
      : await api.createBoard(payload);
    ui.setSuccess(`已保存开发板 ${board.name}`);
    await router.push(`/boards/${board.id}`);
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

function syncBoardIdentity() {
  if (!isEditing.value && idMode.value === "auto") {
    form.value.id = suggestedBoardId.value;
  }
  form.value.name = form.value.id.trim();
}

function enableManualId() {
  idMode.value = "manual";
}

function restoreAutoId() {
  idMode.value = "auto";
  syncBoardIdentity();
}

watch(
  () => [form.value.board_type, knownBoards.value.length, idMode.value, boardId.value] as const,
  () => {
    syncBoardIdentity();
  },
);

watch(
  () => form.value.id,
  () => {
    form.value.name = form.value.id.trim();
  },
  { immediate: true },
);

onMounted(() => {
  void loadBoard();
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
          <button class="ghost-button" @click="loadBoard">重载</button>
          <button class="primary-button" :disabled="saving" @click="saveBoard">
            {{ saving ? "保存中..." : "保存配置" }}
          </button>
        </div>
      </div>

      <div v-if="loading" class="empty-state">正在加载开发板配置...</div>
      <template v-else>
        <p v-if="validationError" class="diagnostic-error">{{ validationError }}</p>

        <div class="form-grid two-columns">
          <section class="form-section">
            <h4>基本信息</h4>
            <label class="field">
              <span>板型</span>
              <input
                v-model="form.board_type"
                list="board-type-options"
                placeholder="输入或选择已有板型"
                required
              />
              <datalist id="board-type-options">
                <option v-for="boardType in knownBoardTypes" :key="boardType" :value="boardType" />
              </datalist>
            </label>
            <label class="field">
              <span>板子 ID</span>
              <div class="inline-field-group">
                <input
                  v-model="form.id"
                  :readonly="isEditing || idMode === 'auto'"
                  :placeholder="suggestedBoardId || '先填写板型'"
                />
                <button
                  v-if="!isEditing && idMode === 'auto'"
                  class="ghost-button compact-button"
                  type="button"
                  @click="enableManualId"
                >
                  手动填写
                </button>
                <button
                  v-else-if="!isEditing"
                  class="ghost-button compact-button"
                  type="button"
                  @click="restoreAutoId"
                >
                  恢复自动
                </button>
              </div>
              <small class="field-hint">
                {{ isEditing ? "现有开发板不支持修改 ID。" : idMode === "auto" ? `将按板型自动建议：${suggestedBoardId || "等待板型输入"}` : "当前为手动填写模式。" }}
              </small>
            </label>
            <label class="field">
              <span>显示名称</span>
              <input v-model="form.name" readonly />
              <small class="field-hint">显示名称自动跟随板子 ID。</small>
            </label>
            <label class="field">
              <span>标签</span>
              <input v-model="form.tagsText" placeholder="lab, usb, arm64" />
            </label>
            <label class="field">
              <span>备注</span>
              <textarea v-model="form.notes" rows="3" placeholder="补充说明" />
            </label>
            <label class="checkbox-field">
              <input v-model="form.disabled" type="checkbox" />
              <span>禁用该开发板</span>
            </label>
          </section>

          <section class="form-section">
            <h4>串口配置</h4>
            <label class="checkbox-field">
              <input v-model="form.serialEnabled" type="checkbox" />
              <span>启用串口</span>
            </label>
            <template v-if="form.serialEnabled">
              <label class="field">
                <span>串口设备</span>
                <div class="inline-field-group">
                  <select v-model="form.serialPort">
                    <option value="" disabled>请选择可用串口</option>
                    <option
                      v-for="port in serialPortOptions"
                      :key="port.port_name"
                      :value="port.port_name"
                    >
                      {{ port.label }}
                    </option>
                  </select>
                  <button class="ghost-button compact-button" type="button" @click="refreshSerialPorts">
                    刷新串口
                  </button>
                </div>
                <small class="field-hint">
                  {{ serialPortOptions.length > 0 ? "优先从系统检测到的串口中选择。" : "当前未检测到可用串口，请先连接设备后刷新。" }}
                </small>
              </label>
              <label class="field">
                <span>波特率</span>
                <input v-model.number="form.serialBaudRate" type="number" min="1" />
              </label>
            </template>

            <h4>启动方式</h4>
            <div class="radio-group">
              <label class="radio-card">
                <input v-model="form.bootKind" value="uboot" type="radio" />
                <span>U-Boot</span>
              </label>
              <label class="radio-card">
                <input v-model="form.bootKind" value="pxe" type="radio" />
                <span>PXE 占位</span>
              </label>
            </div>
          </section>
        </div>

        <section v-if="form.bootKind === 'uboot'" class="form-section">
          <h4>U-Boot 启动配置</h4>
          <div class="form-grid two-columns">
            <label class="checkbox-field">
              <input v-model="form.uboot.use_tftp" type="checkbox" />
              <span>使用 TFTP 启动</span>
            </label>
            <label class="field">
              <span>TFTP 说明</span>
              <small class="field-hint">
                {{
                  form.uboot.use_tftp
                    ? "该板会从 Server 的 TFTP 服务级网络配置中获取 serverip、网关和网卡信息。"
                    : "该板不走网络启动；TFTP 相关参数由 Server 忽略。"
                }}
              </small>
            </label>
            <label class="field">
              <span>FIT 加载地址</span>
              <input v-model="form.uboot.fit_load_addr" placeholder="例如 0x90000000" />
            </label>
            <label class="field">
              <span>内核加载地址</span>
              <input v-model="form.uboot.kernel_load_addr" placeholder="例如 0x80200000" />
            </label>
            <label class="field">
              <span>超时（秒）</span>
              <input v-model="form.uboot.timeout" type="number" min="0" placeholder="留空为无" />
            </label>
            <label class="field">
              <span>板子复位命令</span>
              <input v-model="form.uboot.board_reset_cmd" />
            </label>
            <label class="field">
              <span>板子关机命令</span>
              <input v-model="form.uboot.board_power_off_cmd" />
            </label>
            <label class="field">
              <span>Shell Prefix</span>
              <input v-model="form.uboot.shell_prefix" />
            </label>
            <label class="field">
              <span>Shell Init Command</span>
              <input v-model="form.uboot.shell_init_cmd" />
            </label>
          </div>

          <div class="form-grid three-columns">
            <label class="field">
              <span>成功匹配（每行一条）</span>
              <textarea v-model="form.uboot.success_regex_text" rows="6" />
            </label>
            <label class="field">
              <span>失败匹配（每行一条）</span>
              <textarea v-model="form.uboot.fail_regex_text" rows="6" />
            </label>
            <label class="field">
              <span>预置 U-Boot 命令（每行一条）</span>
              <textarea v-model="form.uboot.uboot_cmd_text" rows="6" />
            </label>
          </div>
        </section>

        <section v-else class="form-section">
          <h4>PXE 占位配置</h4>
          <label class="field">
            <span>说明</span>
            <textarea v-model="form.pxe.notes" rows="4" placeholder="记录未来 PXE 方案" />
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

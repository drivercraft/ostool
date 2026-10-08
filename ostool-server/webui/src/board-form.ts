import type {
  AdminBoardUpsertRequest,
  BoardConfig,
  BootConfig,
  PowerManagementConfig,
  SerialPortKeyKind,
  UbootNetworkMode,
} from "@/types/api";
type PowerManagementKind = "custom" | "zhongsheng_relay" | "qemu";
type BootKind = "uboot" | "pxe" | "httpboot";

export interface BoardEditorFormState {
  id: string;
  board_type: string;
  tags_text: string;
  notes: string;
  disabled: boolean;
  serial_enabled: boolean;
  serial_key_kind: SerialPortKeyKind;
  serial_key_value: string;
  serial_baud_rate: number;
  axloader_serial_parameters_enabled: boolean;
  axloader_baud_rate: number;
  axloader_data_bits: number;
  axloader_parity: "none" | "odd" | "even" | "mark" | "space";
  axloader_stop_bits: "one" | "one_point_five" | "two";
  axloader_flow_control: "none" | "rts_cts";
  power_management_kind: PowerManagementKind;
  power_on_cmd: string;
  power_off_cmd: string;
  relay_serial_key_kind: SerialPortKeyKind;
  relay_serial_key_value: string;
  virtual_device_id: string;
  boot_kind: BootKind;
  boot_arch: string;
  network_mac: string;
  use_tftp: boolean;
  dtb_name: string;
  kernel_load_addr: string;
  fit_load_addr: string;
  bootm_addr: string;
  network_mode: UbootNetworkMode;
  board_ip: string;
  server_ip: string;
  netmask: string;
  gatewayip: string;
  pxe_notes: string;
}

const DEFAULT_SERIAL_BAUD_RATE = 115_200;

export function defaultFormState(): BoardEditorFormState {
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
    axloader_serial_parameters_enabled: false,
    axloader_baud_rate: DEFAULT_SERIAL_BAUD_RATE,
    axloader_data_bits: 8,
    axloader_parity: "none",
    axloader_stop_bits: "one",
    axloader_flow_control: "none",
    power_management_kind: "custom",
    power_on_cmd: "",
    power_off_cmd: "",
    relay_serial_key_kind: "serial_number",
    relay_serial_key_value: "",
    virtual_device_id: "",
    boot_kind: "uboot",
    boot_arch: "",
    network_mac: "",
    use_tftp: false,
    dtb_name: "",
    kernel_load_addr: "",
    fit_load_addr: "",
    bootm_addr: "",
    network_mode: "dhcp",
    board_ip: "",
    server_ip: "",
    netmask: "",
    gatewayip: "",
    pxe_notes: "",
  };
}

export function boardToFormState(board: BoardConfig): BoardEditorFormState {
  const next = defaultFormState();
  next.id = board.id;
  next.board_type = board.board_type;
  next.tags_text = board.tags.join(", ");
  next.notes = board.notes ?? "";
  next.disabled = board.disabled;
  next.network_mac = board.network_identity?.mac_address ?? "";

  if (board.serial) {
    next.serial_enabled = true;
    next.serial_key_kind = board.serial.key.kind;
    next.serial_key_value = board.serial.key.value;
    next.serial_baud_rate = board.serial.baud_rate;
  }

  if (board.boot.kind === "httpboot" && board.boot.serial_parameters) {
    next.axloader_serial_parameters_enabled = true;
    next.axloader_baud_rate = board.boot.serial_parameters.baud_rate;
    next.axloader_data_bits = board.boot.serial_parameters.data_bits;
    next.axloader_parity = board.boot.serial_parameters.parity;
    next.axloader_stop_bits = board.boot.serial_parameters.stop_bits;
    next.axloader_flow_control = board.boot.serial_parameters.flow_control;
  }

  if (board.power_management.kind === "custom") {
    next.power_management_kind = "custom";
    next.power_on_cmd = board.power_management.power_on_cmd;
    next.power_off_cmd = board.power_management.power_off_cmd;
  } else if (board.power_management.kind === "zhongsheng_relay") {
    next.power_management_kind = "zhongsheng_relay";
    next.relay_serial_key_kind = board.power_management.key.kind;
    next.relay_serial_key_value = board.power_management.key.value;
  } else {
    next.power_management_kind = "qemu";
    next.virtual_device_id = board.power_management.virtual_device_id;
  }

  if (board.boot.kind === "uboot") {
    next.boot_kind = "uboot";
    next.use_tftp = board.boot.use_tftp;
    next.dtb_name = board.boot.dtb_name ?? "";
    next.kernel_load_addr = board.boot.kernel_load_addr ?? "";
    next.fit_load_addr = board.boot.fit_load_addr ?? "";
    next.bootm_addr = board.boot.bootm_addr ?? "";
    next.network_mode = board.boot.network_mode ?? "dhcp";
    next.board_ip = board.boot.board_ip ?? "";
    next.server_ip = board.boot.server_ip ?? "";
    next.netmask = board.boot.netmask ?? "";
    next.gatewayip = board.boot.gatewayip ?? "";
  } else if (board.boot.kind === "pxe") {
    next.boot_kind = "pxe";
    next.pxe_notes = board.boot.notes ?? "";
  } else {
    next.boot_kind = "httpboot";
    next.boot_arch = board.boot.boot_arch ?? "";
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

function buildBootConfig(form: BoardEditorFormState): BootConfig {
  if (form.boot_kind === "uboot") {
    const useStaticIp = form.use_tftp && form.network_mode === "static_ip";
    return {
      kind: "uboot",
      use_tftp: form.use_tftp,
      dtb_name: trimToNull(form.dtb_name),
      kernel_load_addr: trimToNull(form.kernel_load_addr),
      fit_load_addr: trimToNull(form.fit_load_addr),
      bootm_addr: trimToNull(form.bootm_addr),
      network_mode: useStaticIp ? "static_ip" : "dhcp",
      board_ip: useStaticIp ? trimToNull(form.board_ip) : null,
      server_ip: useStaticIp ? trimToNull(form.server_ip) : null,
      netmask: useStaticIp ? trimToNull(form.netmask) : null,
      gatewayip: useStaticIp ? trimToNull(form.gatewayip) : null,
    };
  }

  if (form.boot_kind === "httpboot") {
    return {
      kind: "httpboot",
      boot_arch: trimToNull(form.boot_arch),
      serial_parameters: form.axloader_serial_parameters_enabled
        ? {
            baud_rate: form.axloader_baud_rate,
            data_bits: form.axloader_data_bits,
            parity: form.axloader_parity,
            stop_bits: form.axloader_stop_bits,
            flow_control: form.axloader_flow_control,
          }
        : null,
    };
  }

  return {
    kind: "pxe",
    notes: trimToNull(form.pxe_notes),
  };
}

export function buildPowerManagementConfig(
  form: BoardEditorFormState,
): PowerManagementConfig {
  if (form.power_management_kind === "custom") {
    return {
      kind: "custom",
      power_on_cmd: form.power_on_cmd.trim(),
      power_off_cmd: form.power_off_cmd.trim(),
    };
  }

  if (form.power_management_kind === "zhongsheng_relay") {
    return {
      kind: "zhongsheng_relay",
      key: {
        kind: form.relay_serial_key_kind,
        value: form.relay_serial_key_value.trim(),
      },
    };
  }

  return {
    kind: "qemu",
    virtual_device_id: form.virtual_device_id.trim(),
  };
}

export function buildRequestPayload(
  form: BoardEditorFormState,
): AdminBoardUpsertRequest {
  return {
    id: trimToNull(form.id),
    board_type: form.board_type.trim(),
    tags: splitTags(form.tags_text),
    notes: trimToNull(form.notes),
    disabled: form.disabled,
    serial:
      form.boot_kind !== "httpboot" && form.serial_enabled
        ? {
            key: {
              kind: form.serial_key_kind,
              value: form.serial_key_value.trim(),
            },
            baud_rate: form.serial_baud_rate,
          }
        : null,
    power_management: buildPowerManagementConfig(form),
    boot: buildBootConfig(form),
    network_identity:
      form.boot_kind !== "uboot" && trimToNull(form.network_mac)
        ? { mac_address: form.network_mac.trim() }
        : null,
  };
}

export function validateForm(form: BoardEditorFormState): string {
  const errors: string[] = [];

  if (!form.board_type.trim()) {
    errors.push("board_type 不能为空");
  }
  if (form.id.includes("/") || form.id.includes("\\")) {
    errors.push("板子 ID 不能包含路径分隔符");
  }
  if (
    form.boot_kind !== "httpboot" &&
    form.serial_enabled &&
    !form.serial_key_value.trim()
  ) {
    errors.push("启用串口时必须选择串口设备");
  }
  if (
    form.boot_kind !== "httpboot" &&
    form.serial_enabled &&
    (!Number.isFinite(form.serial_baud_rate) || form.serial_baud_rate <= 0)
  ) {
    errors.push("启用串口时波特率必须大于 0");
  }
  if (form.power_management_kind === "custom") {
    if (!form.power_on_cmd.trim()) {
      errors.push("Custom 电源管理必须填写开机命令");
    }
    if (!form.power_off_cmd.trim()) {
      errors.push("Custom 电源管理必须填写关机命令");
    }
  }
  if (
    form.power_management_kind === "zhongsheng_relay" &&
    !form.relay_serial_key_value.trim()
  ) {
    errors.push("中盛继电模块必须选择串口设备");
  }
  if (form.power_management_kind === "qemu") {
    if (!form.virtual_device_id.trim()) {
      errors.push("QEMU 电源管理必须选择虚拟设备");
    }
    if (form.boot_kind !== "httpboot") {
      errors.push("QEMU 虚拟设备必须使用 HTTPboot");
    }
  }
  if (
    form.boot_kind === "uboot" &&
    form.use_tftp &&
    form.network_mode === "static_ip"
  ) {
    if (!form.board_ip.trim()) {
      errors.push("静态 IP 模式必须填写开发板 IP");
    }
  }
  if (form.boot_kind === "httpboot") {
    const mac = form.network_mac.trim();
    if (!mac) {
      errors.push("HTTPboot 板卡必须绑定 MAC 地址");
    } else if (!/^[0-9a-f]{2}(?::[0-9a-f]{2}){5}$/i.test(mac)) {
      errors.push("MAC 地址必须是六字节冒号格式，例如 02:00:00:00:00:01");
    }
    if (form.axloader_serial_parameters_enabled) {
      if (
        !Number.isFinite(form.axloader_baud_rate) ||
        form.axloader_baud_rate <= 0
      ) {
        errors.push("axloader 串口波特率必须大于 0");
      }
      if (![7, 8].includes(form.axloader_data_bits)) {
        errors.push("axloader 串口数据位必须是 7 或 8");
      }
    }
  }
  return errors.join("\n");
}

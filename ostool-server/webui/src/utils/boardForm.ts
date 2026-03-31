import type {
  BoardConfig,
  BootConfig,
  PxeProfile,
  SerialConfig,
  UbootProfile,
} from "@/types/api";

export interface BoardFormModel {
  id: string;
  name: string;
  board_type: string;
  tagsText: string;
  notes: string;
  disabled: boolean;
  serialEnabled: boolean;
  serialPort: string;
  serialBaudRate: number;
  bootKind: BootConfig["kind"];
  uboot: {
    use_tftp: boolean;
    kernel_load_addr: string;
    fit_load_addr: string;
    board_reset_cmd: string;
    board_power_off_cmd: string;
    success_regex_text: string;
    fail_regex_text: string;
    uboot_cmd_text: string;
    shell_prefix: string;
    shell_init_cmd: string;
    timeout: string;
  };
  pxe: {
    notes: string;
  };
}

export function createDefaultBoardForm(): BoardFormModel {
  return {
    id: "",
    name: "",
    board_type: "",
    tagsText: "",
    notes: "",
    disabled: false,
    serialEnabled: false,
    serialPort: "",
    serialBaudRate: 115200,
    bootKind: "uboot",
    uboot: {
      use_tftp: false,
      kernel_load_addr: "",
      fit_load_addr: "",
      board_reset_cmd: "",
      board_power_off_cmd: "",
      success_regex_text: "",
      fail_regex_text: "",
      uboot_cmd_text: "",
      shell_prefix: "",
      shell_init_cmd: "",
      timeout: "",
    },
    pxe: {
      notes: "",
    },
  };
}

export function boardToForm(board: BoardConfig): BoardFormModel {
  const form = createDefaultBoardForm();
  form.id = board.id;
  form.name = board.name;
  form.board_type = board.board_type;
  form.tagsText = joinTags(board.tags);
  form.notes = board.notes ?? "";
  form.disabled = board.disabled;
  form.serialEnabled = Boolean(board.serial);
  form.serialPort = board.serial?.port ?? "";
  form.serialBaudRate = board.serial?.baud_rate ?? 115200;
  form.bootKind = board.boot.kind;

  if (board.boot.kind === "uboot") {
    form.uboot = {
      use_tftp: board.boot.use_tftp,
      kernel_load_addr: board.boot.kernel_load_addr ?? "",
      fit_load_addr: board.boot.fit_load_addr ?? "",
      board_reset_cmd: board.boot.board_reset_cmd ?? "",
      board_power_off_cmd: board.boot.board_power_off_cmd ?? "",
      success_regex_text: joinLines(board.boot.success_regex),
      fail_regex_text: joinLines(board.boot.fail_regex),
      uboot_cmd_text: joinLines(board.boot.uboot_cmd ?? []),
      shell_prefix: board.boot.shell_prefix ?? "",
      shell_init_cmd: board.boot.shell_init_cmd ?? "",
      timeout: board.boot.timeout == null ? "" : String(board.boot.timeout),
    };
  } else {
    form.pxe.notes = board.boot.notes ?? "";
  }

  return form;
}

export function formToBoard(form: BoardFormModel): BoardConfig {
  const serial: SerialConfig | null = form.serialEnabled
    ? {
        port: form.serialPort.trim(),
        baud_rate: Number(form.serialBaudRate),
      }
    : null;

  const boot: BootConfig =
    form.bootKind === "uboot" ? formToUboot(form.uboot) : formToPxe(form.pxe);

  return {
    id: form.id.trim(),
    name: form.name.trim(),
    board_type: form.board_type.trim(),
    tags: parseTags(form.tagsText),
    serial,
    boot,
    notes: emptyToNull(form.notes),
    disabled: form.disabled,
  };
}

function formToUboot(form: BoardFormModel["uboot"]): UbootProfile {
  return {
    kind: "uboot",
    use_tftp: form.use_tftp,
    kernel_load_addr: emptyToNull(form.kernel_load_addr),
    fit_load_addr: emptyToNull(form.fit_load_addr),
    board_reset_cmd: emptyToNull(form.board_reset_cmd),
    board_power_off_cmd: emptyToNull(form.board_power_off_cmd),
    success_regex: parseLines(form.success_regex_text),
    fail_regex: parseLines(form.fail_regex_text),
    uboot_cmd: parseOptionalLines(form.uboot_cmd_text),
    shell_prefix: emptyToNull(form.shell_prefix),
    shell_init_cmd: emptyToNull(form.shell_init_cmd),
    timeout: form.timeout.trim() === "" ? null : Number(form.timeout),
  };
}

function formToPxe(form: BoardFormModel["pxe"]): PxeProfile {
  return {
    kind: "pxe",
    notes: emptyToNull(form.notes),
  };
}

export function parseTags(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

export function joinTags(tags: string[]): string {
  return tags.join(", ");
}

export function parseLines(value: string): string[] {
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}

export function parseOptionalLines(value: string): string[] | null {
  const lines = parseLines(value);
  return lines.length > 0 ? lines : null;
}

export function joinLines(value: string[]): string {
  return value.join("\n");
}

function emptyToNull(value: string): string | null {
  const trimmed = value.trim();
  return trimmed === "" ? null : trimmed;
}

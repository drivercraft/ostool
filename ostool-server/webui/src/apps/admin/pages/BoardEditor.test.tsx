import "@testing-library/jest-dom/vitest";
import { act, cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, expect, it, vi } from "vitest";
import BoardEditor from "./BoardEditor";
import Boards from "./Boards";
import { admin } from "@/api/events";
import { api } from "@/api/client";
import type {
  BoardConfig,
  LoaderDeviceSummary,
  LoaderUpdateJob,
  SerialPortKey,
  SerialPortSummary,
} from "@/types/api";
vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
);
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  sessionStorage.clear();
});
function mount(
  board?: BoardConfig,
  path?: string,
  loaders: LoaderDeviceSummary[] = [],
  serial: SerialPortSummary[] = [],
) {
  admin.apply({
    epoch: crypto.randomUUID(),
    revision: 1,
    kind: "snapshot",
    data: {
      boards: board ? [board] : [],
      board_editor: { id: board?.id ?? "", board: board ?? null },
      serial,
      loaders,
      dtbs: [],
      power_actions: [],
      ota: { images: [], jobs: [] },
      virtual: { enabled: false, devices: [] },
    },
  });
  render(
    <MemoryRouter
      initialEntries={[
        path ??
          (board ? `/boards/${board.id}` : "/boards/new?mac=02:00:00:00:00:21"),
      ]}
    >
      <Routes>
        <Route path="/boards" element={<Boards />} />
        <Route path="/boards/new" element={<BoardEditor />} />
        <Route path="/boards/:boardId" element={<BoardEditor />} />
      </Routes>
    </MemoryRouter>,
  );
}
it("creates and rebinds discovered devices while showing their live architectures read-only", async () => {
  const first = {
    mac_address: "02:00:00:00:00:21",
    current_mac_address: "02:00:00:00:00:21",
    ip_address: "10.0.0.21",
    arch: "riscv64",
    loader_version: "test",
    hardware: { manufacturer: null, product: null, version: null, serial: null },
    online: true,
    conflict: false,
    bound_board_id: null,
    last_seen_at: "2026-10-01T00:00:00Z",
    current_registration_id: "first-epoch",
  } as LoaderDeviceSummary;
  const second = {
    ...first,
    mac_address: "02:00:00:00:00:22",
    arch: "loongarch64",
  } as LoaderDeviceSummary;
  const create = vi
    .spyOn(api, "createBoard")
    .mockImplementation(async (payload) => {
      const saved = { ...payload, id: "discovered" } as BoardConfig;
      act(() =>
        admin.apply({
          epoch: "saved",
          revision: 1,
          kind: "snapshot",
          data: { boards: [saved], board_editor: { id: saved.id, board: saved }, loaders: [first, second] },
        }),
      );
      return saved;
    });
  const update = vi
    .spyOn(api, "updateBoard")
    .mockImplementation(
      async (id, payload) => ({ ...payload, id }) as BoardConfig,
    );
  mount(undefined, "/boards", [first, second]);
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: /发现设备/ }));
  expect(screen.getByText("riscv64")).toBeVisible();
  expect(screen.getByText("loongarch64")).toBeVisible();
  const createLink = screen.getAllByRole("link", { name: "创建配置" })[0];
  expect(createLink.getAttribute("href")).not.toContain("arch=");
  await user.click(createLink);
  expect(screen.getByLabelText("MAC 地址")).toHaveValue(first.mac_address);
  expect(screen.queryByLabelText("启动架构")).not.toBeInTheDocument();
  expect(screen.queryByText("高级启动设置")).not.toBeInTheDocument();
  expect(screen.getByText(/设备架构 riscv64/)).toBeVisible();
  await user.type(screen.getByLabelText("板型"), "discovered");
  await user.type(screen.getByLabelText("上电命令"), "true");
  await user.type(screen.getByLabelText("下电命令"), "true");
  await user.click(screen.getByRole("button", { name: "保存开发板" }));
  expect(create).toHaveBeenCalledOnce();
  expect(create.mock.calls[0][0]).toMatchObject({
    boot: { kind: "httpboot" },
  });
  expect(create.mock.calls[0][0].boot).not.toHaveProperty("boot_arch");
  await screen.findByRole("button", { name: "删除开发板" });
  const notes = screen.getByLabelText("备注");
  await user.type(notes, "local draft");
  act(() =>
    admin.apply({
      epoch: "architecture-updated",
      revision: 1,
      kind: "snapshot",
      data: { loaders: [{ ...first, arch: "aarch64" }, second] },
    }),
  );
  expect(screen.getByText(/设备架构 aarch64/)).toBeVisible();
  expect(notes).toHaveValue("local draft");
  expect(notes).toHaveFocus();
  await user.click(screen.getByRole("button", { name: "发现 2 台设备" }));
  await user.click(
    await screen.findByRole("button", { name: new RegExp(second.mac_address) }),
  );
  expect(screen.getByText(/设备架构 loongarch64/)).toBeVisible();
  expect(screen.queryByLabelText("启动架构")).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "保存开发板" }));
  expect(update).toHaveBeenCalledOnce();
  expect(update.mock.calls[0][1]).toMatchObject({
    network_identity: { mac_address: second.mac_address },
    boot: { kind: "httpboot" },
  });
  expect(update.mock.calls[0][1].boot).not.toHaveProperty("boot_arch");
});
function httpBootBoard(): BoardConfig {
  return {
    id: "http-board",
    board_type: "uefi-http",
    tags: [],
    notes: null,
    disabled: false,
    serial: null,
    network_identity: { mac_address: "02:00:00:00:00:31" },
    power_management: {
      kind: "custom",
      power_on_cmd: "true",
      power_off_cmd: "true",
    },
    boot: { kind: "httpboot" },
  };
}
function usbSerialPort(port: number, serialNumber: string): SerialPortSummary {
  const usbId = `/dev/serial/by-id/ftdi-if0${port}`;
  return {
    current_device_path: `/dev/ttyUSB${port}`,
    port_type: "usb",
    label: "FTDI",
    primary_key_kind: "usb_id",
    primary_key_value: usbId,
    usb_id: usbId,
    usb_path: `/dev/serial/by-path/ftdi-if0${port}`,
    serial_number: serialNumber,
    stable_identity: true,
    usb_vendor_id: 0x0403,
    usb_product_id: 0x6011,
    manufacturer: "FTDI",
    product: "FT4232H",
  };
}
function serialBoard(
  serialKey: SerialPortKey,
  relayKey: SerialPortKey,
): BoardConfig {
  return {
    ...httpBootBoard(),
    boot: { kind: "pxe", notes: null },
    serial: { key: serialKey, baud_rate: 115200 },
    power_management: { kind: "zhongsheng_relay", key: relayKey },
  };
}
it.each([
  { kind: "serial_number", field: "serial_number" },
  { kind: "usb_path", field: "usb_path" },
  { kind: "usb_path", field: "usb_id" },
  { kind: "usb_id", field: "usb_path" },
] as const)(
  "keeps saved $kind identities from $field available when USB ID is preferred",
  async ({ kind, field }) => {
    const consolePort = usbSerialPort(0, "console-123");
    const relayPort = usbSerialPort(1, "relay-123");
    const serialKey = { kind, value: consolePort[field]! };
    const relayKey = { kind, value: relayPort[field]! };
    const board = serialBoard(serialKey, relayKey);
    const update = vi
      .spyOn(api, "updateBoard")
      .mockImplementation(
        async (id, payload) => ({ ...payload, id }) as BoardConfig,
      );
    mount(board);
    const serialSelect = screen.getByLabelText("稳定串口");
    const relaySelect = screen.getByLabelText("继电器");
    for (const select of [serialSelect, relaySelect]) {
      expect(
        within(select).getByRole("option", { selected: true }),
      ).toHaveTextContent("当前未检测到");
    }
    const user = userEvent.setup();
    const notes = screen.getByLabelText("备注");
    await user.type(notes, "local serial draft");
    act(() =>
      admin.apply({
        epoch: "serial-arrived",
        revision: 1,
        kind: "snapshot",
        data: { serial: [consolePort, relayPort] },
      }),
    );
    expect(serialSelect).toHaveValue(`${kind}:${serialKey.value}`);
    expect(relaySelect).toHaveValue(`${kind}:${relayKey.value}`);
    for (const select of [serialSelect, relaySelect]) {
      expect(
        within(select).getByRole("option", { selected: true }),
      ).not.toHaveTextContent("当前未检测到");
    }
    expect(screen.getAllByText(/VID:PID 403:6011/)).toHaveLength(2);
    expect(notes).toHaveValue("local serial draft");
    expect(notes).toHaveFocus();
    await user.click(screen.getByRole("button", { name: "保存开发板" }));
    expect(update).toHaveBeenCalledOnce();
    expect(update.mock.calls[0][1]).toMatchObject({
      serial: { key: serialKey },
      power_management: { kind: "zhongsheng_relay", key: relayKey },
    });
  },
);
it("marks shared serial numbers ambiguous until an interface is selected", async () => {
  const consolePort = usbSerialPort(0, "shared-123");
  const relayPort = usbSerialPort(1, "shared-123");
  const sharedKey: SerialPortKey = {
    kind: "serial_number",
    value: "shared-123",
  };
  const board = serialBoard(sharedKey, sharedKey);
  const update = vi
    .spyOn(api, "updateBoard")
    .mockImplementation(
      async (id, payload) => ({ ...payload, id }) as BoardConfig,
    );
  mount(board, undefined, [], [consolePort, relayPort]);
  const serialSelect = screen.getByLabelText("稳定串口");
  const relaySelect = screen.getByLabelText("继电器");
  for (const select of [serialSelect, relaySelect]) {
    const selected = within(select).getByRole("option", { selected: true });
    expect(selected).toHaveTextContent("匹配多个接口");
    expect(selected).toBeDisabled();
  }
  expect(screen.getAllByText(/匹配多个串口接口/)).toHaveLength(2);
  expect(screen.queryByText(/VID:PID/)).not.toBeInTheDocument();
  act(() =>
    admin.apply({
      epoch: "serial-reordered",
      revision: 1,
      kind: "snapshot",
      data: { serial: [relayPort, consolePort] },
    }),
  );
  expect(screen.getAllByText(/匹配多个串口接口/)).toHaveLength(2);
  expect(screen.queryByText(/VID:PID/)).not.toBeInTheDocument();
  const user = userEvent.setup();
  await user.selectOptions(serialSelect, `usb_id:${consolePort.usb_id}`);
  await user.selectOptions(relaySelect, `usb_id:${relayPort.usb_id}`);
  expect(screen.queryByText(/匹配多个串口接口/)).not.toBeInTheDocument();
  expect(screen.getAllByText(/VID:PID 403:6011/)).toHaveLength(2);
  await user.click(screen.getByRole("button", { name: "保存开发板" }));
  expect(update).toHaveBeenCalledOnce();
  expect(update.mock.calls[0][1]).toMatchObject({
    serial: { key: { kind: "usb_id", value: consolePort.usb_id } },
    power_management: { key: { kind: "usb_id", value: relayPort.usb_id } },
  });
});
it("only prefills the MAC and does not require board identity for a power command", async () => {
  const power = vi.spyOn(api, "powerAction").mockResolvedValue({
    id: "one",
    action: "on",
    state: "succeeded",
    message: "ok",
  });
  mount();
  const user = userEvent.setup();
  expect(screen.getByLabelText("MAC 地址")).toHaveValue("02:00:00:00:00:21");
  expect(screen.getByLabelText("板型")).toHaveValue("");
  expect(
    screen.queryByRole("checkbox", { name: "启用串口" }),
  ).not.toBeInTheDocument();
  await user.type(screen.getByLabelText("上电命令"), "true");
  await user.click(screen.getByRole("button", { name: "上电" }));
  expect(power).toHaveBeenCalledOnce();
  expect(power.mock.calls[0][0]).toMatchObject({
    action: "on",
    power_management: { kind: "custom", power_on_cmd: "true" },
  });
  expect(power.mock.calls[0][0]).not.toHaveProperty("board_id");
});
it("preserves typed MAC and input identity while devices arrive", async () => {
  mount();
  const input = screen.getByLabelText("MAC 地址");
  const user = userEvent.setup();
  await user.clear(input);
  await user.type(input, "02:00:00:00:00:");
  act(() =>
    admin.apply({
      epoch: "device-arrived",
      revision: 1,
      kind: "snapshot",
      data: {
        loaders: [
          {
            mac_address: "02:00:00:00:00:22",
            online: true,
            conflict: false,
            bound_board_id: null,
            hardware: { manufacturer: null, product: null, version: null, serial: null },
            ip_address: "10.0.0.2",
            arch: "x86_64",
          } as LoaderDeviceSummary,
        ],
      },
    }),
  );
  expect(screen.getByLabelText("MAC 地址")).toBe(input);
  expect(input).toHaveValue("02:00:00:00:00:");
  expect(input).toHaveFocus();
  expect(screen.getByRole("button", { name: "发现 1 台设备" })).toBeVisible();
});
it("does not overwrite a local draft when another client edits the board", async () => {
  const board: BoardConfig = {
    id: "one",
    board_type: "arm",
    tags: [],
    notes: null,
    disabled: false,
    serial: null,
    network_identity: null,
    power_management: {
      kind: "custom",
      power_on_cmd: "true",
      power_off_cmd: "true",
    },
    boot: { kind: "pxe", notes: null },
  };
  mount(board);
  const user = userEvent.setup();
  await user.type(screen.getByLabelText("备注"), "draft");
  act(() =>
    admin.apply({
      epoch: "peer",
      revision: 1,
      kind: "snapshot",
      data: {
        board_editor: { id: board.id, board: { ...board, notes: "remote" } },
      },
    }),
  );
  expect(screen.getByLabelText("备注")).toHaveValue("draft");
  expect(screen.getByRole("button", { name: "保存开发板" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "载入服务器版本" }));
  expect(screen.getByLabelText("备注")).toHaveValue("remote");
});

it("switches to U-Boot without requiring or submitting a stale MAC", async () => {
  const create = vi.spyOn(api, "createBoard").mockImplementation(
    async (request) =>
      ({
        ...request,
        id: "created",
        tags: request.tags ?? [],
        boot: request.boot,
      }) as BoardConfig,
  );
  mount();
  const user = userEvent.setup();
  await user.clear(screen.getByLabelText("MAC 地址"));
  await user.type(screen.getByLabelText("MAC 地址"), "unfinished");
  await user.selectOptions(screen.getByLabelText("启动方式"), "uboot");
  expect(screen.queryByLabelText("MAC 地址")).not.toBeInTheDocument();
  await user.type(screen.getByLabelText("板型"), "arm");
  await user.type(screen.getByLabelText("上电命令"), "true");
  await user.type(screen.getByLabelText("下电命令"), "true");
  await user.click(screen.getByRole("button", { name: "保存开发板" }));
  expect(create).toHaveBeenCalledWith(
    expect.objectContaining({
      boot: expect.objectContaining({ kind: "uboot" }),
      network_identity: null,
    }),
  );
});

it.each([
  { arch: "aarch64", online: true, legacyArch: "x86_64" },
  { arch: "x86_64", online: true, legacyArch: "aarch64" },
  { arch: "riscv64", online: false, legacyArch: "aarch64" },
  { arch: null, online: false, legacyArch: "aarch64" },
])(
  "shows reported $arch/$online read-only without limiting OTA actions",
  async ({ arch, online, legacyArch }) => {
    const board = {
      ...httpBootBoard(),
      boot: { kind: "httpboot" as const, boot_arch: legacyArch },
    };
    const loaders = arch
      ? [
          {
            mac_address: board.network_identity!.mac_address,
            ip_address: "10.0.0.31",
            arch,
            hardware: { manufacturer: null, product: null, version: null, serial: null },
            online,
            bound_board_id: board.id,
          } as LoaderDeviceSummary,
        ]
      : [];
    mount(board, undefined, loaders);

    expect(screen.queryByLabelText("启动架构")).not.toBeInTheDocument();
    expect(
      screen.getByText(
        online ? new RegExp(`设备架构 ${arch}`) : /设备架构 未知/,
      ),
    ).toBeVisible();
    expect(screen.getByText("axloader OTA")).toBeVisible();
    expect(screen.getByLabelText("EFI 镜像（最多 32 MiB）")).toBeVisible();
    expect(screen.getByRole("button", { name: "上传镜像" })).toBeVisible();

    const previous: LoaderUpdateJob = {
      board_id: board.id,
      mac_address: board.network_identity!.mac_address,
      update_id: "previous-update",
      image: {
        sha256: "11".repeat(32),
        size: 512,
        version: "previous-loader",
      },
      phase: "failed",
      error: "422 Unprocessable Entity: device rejected image Machine",
      delivery_attempts: 1,
    };
    act(() =>
      admin.apply({
        epoch: "ota-history",
        revision: 1,
        kind: "snapshot",
        data: { ota: { images: [previous.image], jobs: [previous] } },
      }),
    );
    expect(screen.getByText(/previous-update.*failed/)).toBeVisible();
    expect(screen.getByText(previous.error!)).toBeVisible();

    const queued: LoaderUpdateJob = {
      ...previous,
      update_id: "new-update",
      phase: "queued",
      error: "503 Service Unavailable: device has no writable inactive slot",
      next_delivery_at: null,
      waiting_for_device_change: true,
    };
    const queue = vi.spyOn(api, "queueLoaderUpdate").mockResolvedValue(queued);
    const cancel = vi.spyOn(api, "cancelLoaderUpdate").mockResolvedValue({
      ...queued,
      phase: "cancelled",
    });
    const user = userEvent.setup();
    await user.selectOptions(
      screen.getByLabelText("待下发镜像"),
      previous.image.sha256,
    );
    await user.click(screen.getByRole("button", { name: "下发升级" }));
    expect(queue).toHaveBeenCalledWith(board.id, previous.image.sha256);

    // Device discovery must preserve the offline assignment and its controls.
    if (!online && arch) {
      act(() =>
        admin.apply({
          epoch: "loader-online",
          revision: 1,
          kind: "snapshot",
          data: {
            loaders: loaders.map((loader) => ({ ...loader, online: true })),
          },
        }),
      );
      expect(screen.getByText(/previous-update.*failed/)).toBeVisible();
      expect(screen.getByText(previous.error!)).toBeVisible();
      expect(screen.getByRole("button", { name: "下发升级" })).toBeEnabled();
      expect(screen.getByLabelText("EFI 镜像（最多 32 MiB）")).toBeVisible();
    }
    act(() =>
      admin.apply({
        epoch: "ota-waiting",
        revision: 1,
        kind: "snapshot",
        data: { ota: { images: [previous.image], jobs: [queued] } },
      }),
    );
    expect(screen.getByText(/new-update.*queued/)).toBeVisible();
    expect(screen.getByText(queued.error!)).toBeVisible();
    expect(screen.getByRole("button", { name: "下发升级" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "取消升级" }));
    await user.click(screen.getByRole("button", { name: "确认" }));
    expect(cancel).toHaveBeenCalledWith(board.id, queued.update_id);
  },
);

it("loads and edits the optional axloader serial override", async () => {
  const board = httpBootBoard();
  board.boot = {
    kind: "httpboot",
    serial_parameters: {
      baud_rate: 921600,
      data_bits: 7,
      parity: "even",
      stop_bits: "two",
      flow_control: "rts_cts",
    },
  };
  mount(board);
  const user = userEvent.setup();

  expect(screen.getByRole("checkbox", { name: "指定串口参数" })).toBeChecked();
  expect(screen.getByLabelText("波特率")).toHaveValue(921600);
  expect(screen.getByLabelText("数据位")).toHaveValue("7");
  await user.selectOptions(screen.getByLabelText("数据位"), "8");
  expect(screen.getByLabelText("数据位")).toHaveValue("8");
});

it("switches manual UART draft to axloader and preserves focus on serial SSE updates", async () => {
  const board = httpBootBoard();
  board.boot = {
    kind: "uboot",
    use_tftp: false,
    dtb_name: null,
    kernel_load_addr: null,
    fit_load_addr: null,
    bootm_addr: null,
    network_mode: "dhcp",
    board_ip: null,
    server_ip: null,
    netmask: null,
    gatewayip: null,
  };
  board.serial = {
    key: { kind: "usb_path", value: "/dev/serial/by-path/demo" },
    baud_rate: 57600,
  };
  mount(board);
  const user = userEvent.setup();
  expect(screen.getByLabelText("波特率")).toHaveValue(57600);
  await user.selectOptions(screen.getByLabelText("启动方式"), "httpboot");
  expect(screen.queryByLabelText("波特率")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("checkbox", { name: "启用串口" }),
  ).not.toBeInTheDocument();
  const input = screen.getByLabelText("备注");
  await user.type(input, "local draft");
  act(() =>
    admin.apply({
      epoch: "serial-state",
      revision: 1,
      kind: "snapshot",
      data: { serial_manager: { pending: 2, candidates: 4, leased: 1 } },
    }),
  );
  expect(screen.getByText("发现请求 2 · 候选监听 4")).toBeVisible();
  expect(input).toHaveFocus();
  expect(input).toHaveValue("local draft");
  await user.selectOptions(screen.getByLabelText("启动方式"), "uboot");
  expect(screen.getByLabelText("波特率")).toHaveValue(57600);
});

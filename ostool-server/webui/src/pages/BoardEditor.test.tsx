import "@testing-library/jest-dom/vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, expect, it, vi } from "vitest";
import BoardEditor from "./BoardEditor";
import { admin } from "@/api/events";
import { api } from "@/api/client";
import type { BoardConfig, LoaderDeviceSummary } from "@/types/api";
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
function mount(board?: BoardConfig) {
  admin.apply({
    epoch: crypto.randomUUID(),
    revision: 1,
    kind: "snapshot",
    data: {
      boards: board ? [board] : [],
      serial: [],
      loaders: [],
      dtbs: [],
      power_actions: [],
      virtual: { enabled: false, devices: [] },
    },
  });
  render(
    <MemoryRouter
      initialEntries={[
        board ? `/boards/${board.id}` : "/boards/new?mac=02:00:00:00:00:21",
      ]}
    >
      <Routes>
        <Route path="/boards/new" element={<BoardEditor />} />
        <Route path="/boards/:boardId" element={<BoardEditor />} />
      </Routes>
    </MemoryRouter>,
  );
}
function httpBootBoard(bootArch?: string): BoardConfig {
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
    boot: { kind: "httpboot", boot_arch: bootArch },
  };
}
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
  expect(screen.getByRole("checkbox", { name: "启用串口" })).not.toBeChecked();
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
            hardware: {},
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
      data: { boards: [{ ...board, notes: "remote" }] },
    }),
  );
  expect(screen.getByLabelText("备注")).toHaveValue("draft");
  expect(screen.getByRole("button", { name: "保存开发板" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "载入服务器版本" }));
  expect(screen.getByLabelText("备注")).toHaveValue("remote");
});

it("switches to U-Boot without requiring or submitting a stale MAC", async () => {
  const create = vi
    .spyOn(api, "createBoard")
    .mockResolvedValue({} as BoardConfig);
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

it("hides loader OTA for an unsupported HTTP Boot architecture", () => {
  mount(httpBootBoard("aarch64"));

  expect(screen.queryByText("axloader OTA")).not.toBeInTheDocument();
});

it("shows loader OTA when the HTTP Boot architecture is omitted", () => {
  mount(httpBootBoard());

  expect(screen.getByText("axloader OTA")).toBeInTheDocument();
});

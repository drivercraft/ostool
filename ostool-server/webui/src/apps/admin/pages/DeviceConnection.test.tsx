import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DeviceConnection } from "./DeviceConnection";
import { admin } from "@/api/events";
import { api } from "@/api/client";
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
function publish(endpoint: string) {
  admin.apply({
    epoch: crypto.randomUUID(),
    revision: 1,
    kind: "snapshot",
    data: {
      board_editor: {
        id: "tls",
        board: null,
        device_connection: { endpoint, ca_pem: null, configured: true },
      },
    },
  });
}
it("never displays the stored credential and keeps unsaved connection settings across SSE changes", async () => {
  publish("https://device.test");
  render(<DeviceConnection boardId="tls" />);
  const user = userEvent.setup();
  const endpoint = screen.getByLabelText("设备 HTTPS 地址") as HTMLInputElement;
  const token = screen.getByLabelText("设备独立凭据") as HTMLInputElement;
  expect(token.value).toBe("");
  expect(token.type).toBe("password");
  await user.clear(endpoint);
  await user.type(endpoint, "https://draft.test");
  act(() => publish("https://updated.test"));
  expect(endpoint.value).toBe("https://draft.test");
  expect(
    (screen.getByRole("button", { name: "保存设备认证" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  await user.click(screen.getByRole("button", { name: "载入最新配置" }));
  expect(endpoint.value).toBe("https://updated.test");
});
it("preserves a stored credential when the replacement is left blank", async () => {
  publish("https://device.test");
  const save = vi.spyOn(api, "saveDeviceConnection").mockResolvedValue({
    endpoint: "https://device.test",
    ca_pem: null,
    configured: true,
  });
  render(<DeviceConnection boardId="tls" />);
  await userEvent
    .setup()
    .click(screen.getByRole("button", { name: "保存设备认证" }));
  expect(save).toHaveBeenCalledWith("tls", {
    endpoint: "https://device.test",
    token: undefined,
    ca_pem: null,
  });
});

it("accepts its own save when SSE and HTTP serialize fields in different orders", async () => {
  publish("https://device.test");
  vi.spyOn(api, "saveDeviceConnection").mockResolvedValue({
    endpoint: "https://device.test",
    ca_pem: null,
    configured: true,
  });
  render(<DeviceConnection boardId="tls" />);
  await userEvent
    .setup()
    .click(screen.getByRole("button", { name: "保存设备认证" }));
  act(() =>
    admin.apply({
      epoch: crypto.randomUUID(),
      revision: 1,
      kind: "snapshot",
      data: {
        board_editor: {
          id: "tls",
          board: null,
          device_connection: {
            ca_pem: null,
            configured: true,
            endpoint: "https://device.test",
          },
        },
      },
    }),
  );
  expect(screen.queryByText("服务器认证配置已更新，输入已保留。")).toBeNull();
  expect(
    (screen.getByRole("button", { name: "保存设备认证" }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
});

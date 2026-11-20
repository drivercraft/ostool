import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { toast } from "sonner";
import { EnableAction } from "./enable-action";
import { PermissionsProvider } from "./permissions";

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

it("sends an explicit state, prevents duplicate clicks and waits for SSE to change the label", async () => {
  let finish!: () => void;
  const action = vi.fn(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  const view = render(
    <EnableAction
      enabled
      name="开发板 demo"
      permission="devices.manage"
      action={action}
    />,
  );
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "禁用" }));
  await user.click(screen.getByRole("button", { name: "禁用" }));
  expect(action).toHaveBeenCalledTimes(1);
  expect(action).toHaveBeenCalledWith(false);
  await act(async () => finish());
  expect(toast.success).toHaveBeenCalledWith("开发板 demo已禁用");
  expect(screen.getByRole("button", { name: "禁用" })).toBeTruthy();
  view.rerender(
    <EnableAction
      enabled={false}
      name="开发板 demo"
      permission="devices.manage"
      action={action}
    />,
  );
  await user.click(screen.getByRole("button", { name: "启用" }));
  expect(action).toHaveBeenLastCalledWith(true);
  await act(async () => finish());
});

it("keeps the old state after a rejected mutation and respects manage permissions", async () => {
  const action = vi.fn().mockRejectedValue(new Error("请先配置设备认证"));
  const view = render(
    <EnableAction
      enabled={false}
      name="开发板 demo"
      permission="devices.manage"
      action={action}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "启用" }));
  await waitFor(() =>
    expect(toast.error).toHaveBeenCalledWith("请先配置设备认证"),
  );
  expect(screen.getByRole("button", { name: "启用" })).toBeTruthy();
  view.rerender(
    <PermissionsProvider local={false}>
      <EnableAction
        enabled={false}
        name="开发板 demo"
        permission="devices.manage"
        action={action}
      />
    </PermissionsProvider>,
  );
  expect(screen.queryByRole("button", { name: "启用" })).toBeNull();
});

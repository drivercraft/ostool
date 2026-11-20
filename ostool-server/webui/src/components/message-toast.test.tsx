import { cleanup, render } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { toast } from "sonner";
import { MessageToast } from "./message-toast";

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn(), warning: vi.fn(), info: vi.fn() },
}));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

it("ignores empty feedback and does not replay an unchanged subscription error", () => {
  const view = render(<MessageToast message="" />);
  expect(toast.error).not.toHaveBeenCalled();
  view.rerender(<MessageToast message="连接失败" />);
  view.rerender(<MessageToast message="连接失败" />);
  expect(toast.error).toHaveBeenCalledWith("连接失败", {
    id: "error:连接失败",
  });
  expect(toast.error).toHaveBeenCalledTimes(1);
  expect(view.container.textContent).toBe("");
});

it("uses the same ID across error consumers and distinguishes successful feedback", () => {
  render(
    <>
      <MessageToast message="连接失败" />
      <MessageToast message="连接失败" />
      <MessageToast kind="success" message="已保存" />
    </>,
  );
  expect(toast.error).toHaveBeenNthCalledWith(1, "连接失败", {
    id: "error:连接失败",
  });
  expect(toast.error).toHaveBeenNthCalledWith(2, "连接失败", {
    id: "error:连接失败",
  });
  expect(toast.success).toHaveBeenCalledTimes(1);
  expect(toast.success).toHaveBeenCalledWith("已保存", {
    id: "success:已保存",
  });
});

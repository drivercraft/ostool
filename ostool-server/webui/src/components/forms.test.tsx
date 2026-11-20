import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  ConfirmAction,
  FormGrid,
  ReloadNotice,
  TextField,
  useAction,
} from "./forms";
import { toast } from "sonner";

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

it("reloading server data never submits the surrounding editor form", async () => {
  const reload = vi.fn();
  const submit = vi.fn((event: React.FormEvent) => event.preventDefault());
  render(
    <FormGrid asChild>
      <form onSubmit={submit}>
        <TextField label="草稿" value="尚未保存的输入" readOnly />
        <ReloadNotice onReload={reload}>
          服务器已有新版本，草稿已保留。
        </ReloadNotice>
      </form>
    </FormGrid>,
  );
  await userEvent
    .setup()
    .click(screen.getByRole("button", { name: "载入服务器版本" }));
  expect(reload).toHaveBeenCalledOnce();
  expect(submit).not.toHaveBeenCalled();
  expect((screen.getByLabelText("草稿") as HTMLInputElement).value).toBe(
    "尚未保存的输入",
  );
});

it("reports a failed dialog action once through the popup and keeps the dialog open", async () => {
  render(
    <ConfirmAction
      label="删除配置"
      description="确定删除？"
      action={async () => {
        throw new Error("删除失败");
      }}
    />,
  );
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "删除配置" }));
  await user.click(screen.getByRole("button", { name: "确认" }));
  await waitFor(() => expect(toast.error).toHaveBeenCalledWith("删除失败"));
  expect(toast.error).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("dialog")).toBeTruthy();
  expect(screen.queryByText("删除失败")).toBeNull();
  expect(toast.success).not.toHaveBeenCalled();
});

it("reports success only after an action resolves and allows read-only actions to opt out", async () => {
  let finish!: () => void;
  function Action({ message }: { message?: string | false }) {
    const task = useAction();
    return (
      <button
        onClick={() =>
          void task.run(
            () =>
              new Promise<void>((resolve) => {
                finish = resolve;
              }),
            message,
          )
        }
      >
        提交
      </button>
    );
  }
  const view = render(<Action />);
  await userEvent.click(screen.getByRole("button", { name: "提交" }));
  expect(toast.success).not.toHaveBeenCalled();
  await act(async () => finish());
  await waitFor(() => expect(toast.success).toHaveBeenCalledWith("操作成功"));
  expect(toast.success).toHaveBeenCalledTimes(1);
  vi.clearAllMocks();
  view.rerender(<Action message={false} />);
  await userEvent.click(screen.getByRole("button", { name: "提交" }));
  await act(async () => finish());
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "提交" })).toBeTruthy(),
  );
  expect(toast.success).not.toHaveBeenCalled();
});

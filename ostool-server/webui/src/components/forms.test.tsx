import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { FormGrid, ReloadNotice, TextField } from "./forms";

afterEach(cleanup);

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

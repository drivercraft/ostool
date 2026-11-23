import "@testing-library/jest-dom/vitest";
import { act, cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { afterEach, expect, it, vi } from "vitest";
import { admin } from "@/api/events";
import { api } from "@/api/client";
import type { DtbFileResponse } from "@/types/api";
import Dtbs from "./Dtbs";
vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    disconnect() {}
  },
);
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  sessionStorage.clear();
});
const file: DtbFileResponse = {
  name: "board.dtb",
  disabled: false,
  size: 12,
  updated_at: "2026-11-01T00:00:00Z",
  relative_tftp_path_template: "boot/dtb/board.dtb",
};
function push(value: DtbFileResponse) {
  admin.apply({
    epoch: crypto.randomUUID(),
    revision: 1,
    kind: "snapshot",
    data: { dtbs: [value] },
  });
}
it("shows edit, toggle and more, and updates the toggle from SSE", async () => {
  const disable = vi
    .spyOn(api, "setDtbDisabled")
    .mockResolvedValue({ ...file, disabled: true });
  push(file);
  render(
    <MemoryRouter>
      <Dtbs />
    </MemoryRouter>,
  );
  const row = screen.getByRole("row", { name: /board.dtb/ });
  expect(within(row).getAllByRole("button")).toHaveLength(3);
  expect(within(row).getByRole("button", { name: "修改" })).toBeVisible();
  const user = userEvent.setup();
  await user.click(within(row).getByRole("button", { name: "禁用" }));
  expect(disable).toHaveBeenCalledWith(file.name, true);
  act(() => push({ ...file, disabled: true }));
  expect(within(row).getByText("已禁用")).toBeVisible();
  expect(within(row).getByRole("button", { name: "启用" })).toBeVisible();
  await user.click(within(row).getByRole("button", { name: "更多" }));
  expect(screen.getByRole("menuitem", { name: "删除" })).toBeVisible();
});

import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "@/api/client";
import { ForceReleaseAction } from "./force-release-action";
import { PermissionsProvider } from "./permissions";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
it("allows explicit forced termination and explains that the device remains disabled", async () => {
  const remove = vi.spyOn(api, "deleteSession").mockResolvedValue(undefined);
  render(
    <ForceReleaseAction sessionId="failed-session" boardId="offline-board" />,
  );
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "强制释放" }));
  expect(remove).not.toHaveBeenCalled();
  expect(screen.getByRole("dialog").textContent).toContain("不等待设备恢复");
  expect(screen.getByRole("dialog").textContent).toContain("将禁用");
  await user.click(screen.getByRole("button", { name: "确认" }));
  await waitFor(() =>
    expect(remove).toHaveBeenCalledWith("failed-session", true),
  );
});
it("does not expose forced termination to read-only users", () => {
  render(
    <PermissionsProvider local={false}>
      <ForceReleaseAction sessionId="session" boardId="board" />
    </PermissionsProvider>,
  );
  expect(screen.queryByRole("button", { name: "强制释放" })).toBeNull();
});

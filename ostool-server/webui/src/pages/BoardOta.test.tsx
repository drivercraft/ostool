import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";

import { api } from "@/api/client";
import { admin } from "@/api/events";
import type { LoaderOtaResources, LoaderUpdateJob } from "@/types/api";
import { BoardOta } from "./BoardOta";

const image = {
  sha256: "11".repeat(32),
  size: 512,
  version: "test-loader",
};

function applyOta(job: LoaderUpdateJob) {
  const ota: LoaderOtaResources = { images: [image], jobs: [job] };
  admin.apply({
    epoch: crypto.randomUUID(),
    revision: 1,
    kind: "snapshot",
    data: { ota },
  });
}

function job(phase: LoaderUpdateJob["phase"]): LoaderUpdateJob {
  return {
    board_id: "other-board",
    mac_address: "02:00:00:00:00:01",
    update_id: "update-1",
    image,
    phase,
    error: null,
    delivery_attempts: 0,
  };
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

it("disables deletion while any active assignment references the image", async () => {
  applyOta(job("downloading"));
  render(<BoardOta boardId="board-1" />);

  await userEvent.setup().selectOptions(
    screen.getByLabelText("待下发镜像"),
    image.sha256,
  );
  expect(screen.getByRole("button", { name: "删除镜像" })).toBeDisabled();
});

it("deletes an image referenced only by a terminal assignment", async () => {
  applyOta(job("succeeded"));
  const remove = vi.spyOn(api, "deleteLoaderImage").mockResolvedValue(undefined);
  render(<BoardOta boardId="board-1" />);
  const user = userEvent.setup();
  const select = screen.getByLabelText("待下发镜像");

  await user.selectOptions(select, image.sha256);
  await user.click(screen.getByRole("button", { name: "删除镜像" }));
  await user.click(screen.getByRole("button", { name: "确认" }));

  await waitFor(() => expect(remove).toHaveBeenCalledWith(image.sha256));
  await waitFor(() => expect(select).toHaveValue(""));
});

it("allows selecting the same image after a successful upload", async () => {
  applyOta(job("succeeded"));
  const upload = vi.spyOn(api, "uploadLoaderImage").mockResolvedValue(image);
  render(<BoardOta boardId="board-1" />);
  const user = userEvent.setup();
  const input = screen.getByLabelText("EFI 镜像（最多 32 MiB）");
  const button = screen.getByRole("button", { name: "上传镜像" });
  const file = new File(["efi"], "loader.efi", {
    type: "application/octet-stream",
  });

  await user.upload(input, file);
  await user.click(button);
  await waitFor(() => expect(upload).toHaveBeenCalledTimes(1));
  expect(input).toHaveValue("");

  await user.upload(input, file);
  await user.click(button);
  await waitFor(() => expect(upload).toHaveBeenCalledTimes(2));
});

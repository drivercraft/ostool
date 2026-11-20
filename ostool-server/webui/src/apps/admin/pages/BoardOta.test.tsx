import "@testing-library/jest-dom/vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
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

  await userEvent
    .setup()
    .selectOptions(screen.getByLabelText("待下发镜像"), image.sha256);
  expect(screen.getByRole("button", { name: "删除镜像" })).toBeDisabled();
});

it("deletes an image referenced only by a terminal assignment", async () => {
  applyOta(job("succeeded"));
  const remove = vi
    .spyOn(api, "deleteLoaderImage")
    .mockResolvedValue(undefined);
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
  const input = screen.getByLabelText("上传 EFI 镜像");
  const file = new File(["efi"], "loader.efi", {
    type: "application/octet-stream",
  });

  await user.upload(input, file);
  await waitFor(() => expect(upload).toHaveBeenCalledTimes(1));
  expect(input).toHaveValue("");

  await user.upload(input, file);
  await waitFor(() => expect(upload).toHaveBeenCalledTimes(2));
});

it("shows real upload progress in the image field and keeps the original filename before SSE arrives", async () => {
  applyOta(job("succeeded"));
  let finish!: (value: typeof image) => void;
  vi.spyOn(api, "uploadLoaderImage").mockImplementation(
    (_file, _version, progress) => {
      progress?.(29, 100);
      return new Promise((resolve) => {
        finish = resolve;
      });
    },
  );
  render(<BoardOta boardId="board-1" />);
  await userEvent.upload(
    screen.getByLabelText("上传 EFI 镜像"),
    new File(["efi"], "研发 镜像.efi"),
  );
  expect(
    screen.getByRole("progressbar", { name: "镜像上传进度" }),
  ).toHaveAttribute("aria-valuenow", "29");
  expect(screen.getByText("上传中 29%")).toBeVisible();
  expect(screen.getByRole("button", { name: "上传镜像" })).toBeDisabled();
  await act(async () => finish({ ...image, sha256: "22".repeat(32) }));
  expect(screen.queryByRole("progressbar")).toBeNull();
  expect(screen.getByRole("option", { name: "研发 镜像.efi" })).toHaveProperty(
    "selected",
    true,
  );
  expect(screen.getByRole("button", { name: "下发升级" })).toBeEnabled();
});

it("tracks dispatch stages through SSE without inventing a percentage", async () => {
  applyOta(job("succeeded"));
  const queued = { ...job("queued"), board_id: "board-1" };
  vi.spyOn(api, "queueLoaderUpdate").mockResolvedValue(queued);
  render(<BoardOta boardId="board-1" />);
  const user = userEvent.setup();
  await user.selectOptions(screen.getByLabelText("待下发镜像"), image.sha256);
  await user.click(screen.getByRole("button", { name: "下发升级" }));
  expect(
    screen.getByRole("progressbar", { name: "镜像下发进度" }),
  ).not.toHaveAttribute("aria-valuenow");
  expect(screen.getByText("等待设备接收…")).toBeVisible();
  expect(screen.getByRole("button", { name: "下发升级" })).toBeDisabled();
  act(() => applyOta({ ...queued, phase: "downloading" }));
  expect(screen.getByText("正在下发镜像…")).toBeVisible();
  act(() =>
    applyOta({
      ...queued,
      phase: "downloading",
      waiting_for_device_change: true,
    }),
  );
  expect(screen.getByText("等待设备状态变化…")).toBeVisible();
  act(() => applyOta({ ...queued, phase: "succeeded" }));
  expect(screen.queryByRole("progressbar")).toBeNull();
  expect(screen.getByLabelText("待下发镜像")).toHaveValue(image.sha256);
  expect(screen.getByRole("button", { name: "下发升级" })).toBeEnabled();
});

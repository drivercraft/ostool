import { useEffect, useId, useRef, useState } from "react";
import { toast } from "sonner";
import { api } from "@/api/client";
import { useResource } from "@/api/events";
import { Button } from "@/components/ui/button";
import { Field } from "@/components/ui/field";
import { NativeSelect } from "@/components/ui/native-select";
import {
  Section,
  FieldTitle,
  Notice,
  ConfirmAction,
  useAction,
} from "@/components/forms";
import type { LoaderImage, LoaderUpdateJob } from "@/types/api";
import { sanitizeImageVersion } from "@/utils/ota";

const terminalPhases = ["succeeded", "rolled_back", "failed", "cancelled"];
const phaseLabels: Record<LoaderUpdateJob["phase"], string> = {
  queued: "等待设备接收…",
  downloading: "正在下发镜像…",
  staged: "镜像已写入，等待重启…",
  confirming: "正在确认升级结果…",
  succeeded: "升级成功",
  rolled_back: "升级已回滚",
  failed: "升级失败",
  cancelled: "升级已取消",
};

export function BoardOta({ boardId }: { boardId: string }) {
  const ota = useResource("ota");
  const inputId = useId();
  const fileInput = useRef<HTMLInputElement>(null);
  const [selected, setSelected] = useState("");
  const [uploaded, setUploaded] = useState<{
    image: LoaderImage;
    name: string;
  }>();
  const [uploadPercent, setUploadPercent] = useState<number>();
  const [accepted, setAccepted] = useState<LoaderUpdateJob>();
  const upload = useAction();
  const assign = useAction();
  const liveJob = ota?.jobs.find((entry) => entry.board_id === boardId);
  const job =
    liveJob?.update_id === accepted?.update_id
      ? liveJob
      : (accepted ?? liveJob);
  useEffect(() => {
    if (accepted && liveJob?.update_id === accepted.update_id)
      setAccepted(undefined);
  }, [accepted, liveJob]);
  const pending = !!job && !terminalPhases.includes(job.phase);
  const activeJob = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (pending && job) activeJob.current = job.update_id;
    else if (job && activeJob.current === job.update_id) {
      activeJob.current = undefined;
      if (job.phase === "succeeded") toast.success("镜像升级成功");
      else if (job.phase === "failed" || job.phase === "rolled_back")
        toast.error(job.error || phaseLabels[job.phase]);
    }
  }, [pending, job]);
  useEffect(() => {
    if (pending && job && !selected) setSelected(job.image.sha256);
  }, [pending, job, selected]);
  const busy = upload.pending || assign.pending || pending;
  const images = [...(ota?.images ?? [])];
  for (const image of [uploaded?.image, pending ? job?.image : undefined]) {
    if (image && !images.some((entry) => entry.sha256 === image.sha256))
      images.push(image);
  }
  const selectedImage = images.find((image) => image.sha256 === selected);
  const selectedInUse = ota?.jobs.some(
    (entry) =>
      entry.image.sha256 === selected && !terminalPhases.includes(entry.phase),
  );
  const progressText = upload.pending
    ? uploadPercent === 100
      ? "正在保存镜像…"
      : uploadPercent === undefined
        ? "上传中…"
        : `上传中 ${uploadPercent}%`
    : assign.pending
      ? "正在提交升级任务…"
      : pending && job
        ? job.waiting_for_device_change
          ? "等待设备状态变化…"
          : job.next_delivery_at
            ? "等待重新下发…"
            : phaseLabels[job.phase]
        : undefined;
  const progressValue = upload.pending ? uploadPercent : undefined;

  async function uploadFile(file: File) {
    if (busy) return;
    await upload.run(async () => {
      if (!file.size || file.size > 32 * 1024 * 1024)
        throw new Error("EFI 镜像需为非空文件，且不超过 32 MiB");
      setUploadPercent(0);
      try {
        const image = await api.uploadLoaderImage(
          file,
          sanitizeImageVersion(file.name),
          (loaded, total) => {
            setUploadPercent(
              total > 0
                ? Math.min(100, Math.round((loaded * 100) / total))
                : undefined,
            );
          },
        );
        setUploaded({ image, name: file.name });
        setSelected(image.sha256);
      } finally {
        setUploadPercent(undefined);
      }
    }, "镜像已保存");
  }

  return (
    <Section
      title="axloader OTA"
      hint="仅用于可信隔离实验网；SHA-256 校验不认证发布者。"
    >
      <Field className="full-width">
        <FieldTitle
          htmlFor={inputId}
          label="待下发镜像"
          hint="选择已有镜像，或上传不超过 32 MiB 的 EFI 镜像。"
        />
        <div className="ota-image-controls">
          <div className="ota-image-selection" aria-busy={busy}>
            <NativeSelect
              id={inputId}
              className="w-full"
              value={selected}
              onChange={(event) => setSelected(event.target.value)}
              disabled={busy}
            >
              <option value="">选择或上传 EFI 镜像</option>
              {selected && !selectedImage && (
                <option value={selected} disabled>
                  镜像已不可用
                </option>
              )}
              {images.map((image) => (
                <option key={image.sha256} value={image.sha256}>
                  {image.sha256 === uploaded?.image.sha256
                    ? uploaded.name
                    : image.version || image.sha256.slice(0, 12)}
                </option>
              ))}
            </NativeSelect>
            {progressText && (
              <div
                className="ota-image-progress"
                role="progressbar"
                aria-label={upload.pending ? "镜像上传进度" : "镜像下发进度"}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={progressValue}
                aria-valuetext={progressText}
              >
                <div
                  aria-hidden="true"
                  className={`ota-image-progress-fill${progressValue === undefined ? " is-indeterminate" : ""}`}
                  style={
                    progressValue === undefined
                      ? undefined
                      : { width: `${progressValue}%` }
                  }
                />
                <span>{progressText}</span>
              </div>
            )}
          </div>
          <Button
            permission="ota.manage"
            type="button"
            variant="outline"
            disabled={busy}
            onClick={() => fileInput.current?.click()}
          >
            上传镜像
          </Button>
          <ConfirmAction
            label="删除镜像"
            description="删除选中的 EFI 镜像？已有终态任务会保留镜像摘要，但不能再下载该文件。"
            disabled={!selectedImage || busy || selectedInUse}
            action={async () => {
              await api.deleteLoaderImage(selected);
              if (uploaded?.image.sha256 === selected) setUploaded(undefined);
              setSelected("");
            }}
          />
          <Button
            permission="ota.manage"
            type="button"
            disabled={!selectedImage || busy}
            onClick={() =>
              void assign.run(async () => {
                setAccepted(await api.queueLoaderUpdate(boardId, selected));
              }, "升级任务已指派")
            }
          >
            下发升级
          </Button>
        </div>
        <input
          ref={fileInput}
          type="file"
          hidden
          aria-label="上传 EFI 镜像"
          accept=".efi,application/octet-stream"
          disabled={busy}
          onChange={(event) => {
            const file = event.currentTarget.files?.[0];
            event.currentTarget.value = "";
            if (file) void uploadFile(file);
          }}
        />
      </Field>
      {job && (
        <div className="full-width">
          <p>
            任务 {job.update_id} · {job.phase} · 镜像{" "}
            {job.image.sha256 === uploaded?.image.sha256
              ? uploaded.name
              : job.image.version || job.image.sha256.slice(0, 12)}
          </p>
          {job.error && <Notice>{job.error}</Notice>}
          {pending && (
            <ConfirmAction
              label="取消升级"
              description="取消当前升级任务？设备若已进入待试槽，将在下次复位时回滚。"
              action={async () =>
                setAccepted(
                  await api.cancelLoaderUpdate(boardId, job.update_id),
                )
              }
            />
          )}
        </div>
      )}
    </Section>
  );
}

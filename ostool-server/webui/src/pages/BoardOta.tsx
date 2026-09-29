import { useState } from "react";
import { api } from "@/api/client";
import { useResource } from "@/api/events";
import { Button } from "@/components/ui/button";
import { Section, SelectField, Notice, ConfirmAction, useAction } from "@/components/forms";
import { sanitizeImageVersion } from "@/utils/ota";

const terminalPhases = ["succeeded", "rolled_back", "failed", "cancelled"];

export function BoardOta({ boardId }: { boardId: string }) {
  const ota = useResource("ota");
  const [selected, setSelected] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const upload = useAction();
  const assign = useAction();
  const job = ota?.jobs.find((entry) => entry.board_id === boardId);
  const pending = job && !terminalPhases.includes(job.phase);
  const selectedInUse = ota?.jobs.some(
    (entry) =>
      entry.image.sha256 === selected && !terminalPhases.includes(entry.phase),
  );
  return (
    <Section title="axloader OTA" hint="仅用于可信隔离实验网；SHA-256 校验不认证发布者。">
      {job && (
        <div className="full-width">
          <p>任务 {job.update_id} · {job.phase} · 镜像 {job.image.version || job.image.sha256.slice(0, 12)}</p>
          {job.error && <Notice>{job.error}</Notice>}
          {pending && (
            <ConfirmAction label="取消升级" description="取消当前升级任务？设备若已进入待试槽，将在下次复位时回滚。"
              action={() => api.cancelLoaderUpdate(boardId, job.update_id)} />
          )}
        </div>
      )}
      <label className="full-width">
        EFI 镜像（最多 32 MiB）
        <input type="file" accept=".efi,application/octet-stream"
          onChange={(event) => setFile(event.target.files?.[0] ?? null)} />
      </label>
      <Button type="button" variant="outline" disabled={!file || upload.pending}
        onClick={() => void upload.run(async () => {
          if (!file) return;
          const image = await api.uploadLoaderImage(file, sanitizeImageVersion(file.name));
          setSelected(image.sha256);
          setFile(null);
        }, "镜像已保存")}>上传镜像</Button>
      <SelectField label="待下发镜像" value={selected}
        onValue={setSelected} options={[
          { value: "", label: "选择已上传的镜像" },
          ...(ota?.images ?? []).map((image) => ({
            value: image.sha256,
            label: `${image.version || image.sha256.slice(0, 12)} · ${(image.size / 1024 / 1024).toFixed(2)} MiB`,
          })),
        ]} />
      <ConfirmAction
        label="删除镜像"
        description="删除选中的 EFI 镜像？已有终态任务会保留镜像摘要，但不能再下载该文件。"
        disabled={!selected || selectedInUse}
        action={async () => {
          await api.deleteLoaderImage(selected);
          setSelected("");
        }}
      />
      <Button type="button" disabled={!selected || !!pending || assign.pending}
        onClick={() => void assign.run(() => api.queueLoaderUpdate(boardId, selected), "升级任务已指派")}>
        下发升级
      </Button>
      {(upload.error || assign.error) && <Notice>{upload.error || assign.error}</Notice>}
    </Section>
  );
}

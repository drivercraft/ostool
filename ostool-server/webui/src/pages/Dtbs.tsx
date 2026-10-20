import { ListToolbar } from "@/components/list-toolbar";
import { useRef, useState } from "react";
import { api } from "@/api/client";
import { useList } from "@/api/events";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { FieldGroup } from "@/components/ui/field";
import {
  DataTable,
  Row,
  Cell,
  ConfirmAction,
  TextField,
  Notice,
  useAction,
} from "@/components/forms";
import type { DtbFileResponse } from "@/types/api";
export function validateFile(file: File | null) {
  if (file && file.size > 10 * 1024 * 1024)
    throw new Error("DTB 文件大小不能超过 10 MiB");
}
export function DtbUpload({
  onUploaded,
}: {
  onUploaded?: (name: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const task = useAction();
  return (
    <>
      <Button type="button" variant="outline" onClick={() => setOpen(true)}>
        上传 DTB
      </Button>
      <Dialog
        open={open}
        onOpenChange={(v) => {
          if (!task.pending) setOpen(v);
        }}
      >
        <DialogContent>
          <DialogTitle>上传 DTB</DialogTitle>
          <DialogDescription>单个文件最大 10 MiB。</DialogDescription>
          <FieldGroup>
            <TextField
              label="文件"
              type="file"
              accept=".dtb,application/octet-stream"
              onChange={(e) => {
                const f = e.target.files?.[0] ?? null;
                setFile(f);
                if (f) setName(f.name);
              }}
            />
            <TextField label="文件名" value={name} onValue={setName} />
          </FieldGroup>
          {task.error && <Notice>{task.error}</Notice>}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setOpen(false)}
              disabled={task.pending}
            >
              取消
            </Button>
            <Button
              type="button"
              disabled={task.pending}
              onClick={() =>
                void task.run(async () => {
                  if (!file) throw new Error("请选择 DTB 文件");
                  validateFile(file);
                  const result = await api.createDtb(
                    name.trim() || file.name,
                    file,
                  );
                  onUploaded?.(result.name);
                  setOpen(false);
                  setFile(null);
                  setName("");
                }, "DTB 已上传")
              }
            >
              {task.pending ? "上传中…" : "上传"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
export default function Dtbs() {
  const dtbs = useList("dtbs");
  const [search, setSearch] = useState("");
  const filtered = dtbs.filter((d) =>
    d.name.toLowerCase().includes(search.toLowerCase()),
  );
  const [editing, setEditing] = useState<DtbFileResponse | null>(null);
  return (
    <>
      <header className="page-header">
        <h1>DTB</h1>
      </header>
      <p className="page-description">
        管理板卡启动使用的设备树文件，单个文件最大 10 MiB。
      </p>
      <ListToolbar actions={<DtbUpload />} count={`${filtered.length} 个文件`}>
        <TextField
          label="搜索 DTB"
          value={search}
          onValue={setSearch}
          placeholder="按文件名筛选"
        />
      </ListToolbar>
      <DataTable
        headers={["名称", "大小", "更新时间", "TFTP 路径模板", "操作"]}
        empty={!filtered.length}
      >
        {filtered.map((d) => (
          <Row key={d.name}>
            <Cell className="mono">{d.name}</Cell>
            <Cell>{formatSize(d.size)}</Cell>
            <Cell>{new Date(d.updated_at).toLocaleString()}</Cell>
            <Cell className="mono">{d.relative_tftp_path_template}</Cell>
            <Cell>
              <div className="actions">
                <Button size="sm" variant="ghost" onClick={() => setEditing(d)}>
                  修改
                </Button>
                <ConfirmAction
                  label="删除"
                  description={`删除 DTB ${d.name}？已被板卡引用的文件不能删除。`}
                  action={() => api.deleteDtb(d.name)}
                />
              </div>
            </Cell>
          </Row>
        ))}
      </DataTable>
      {editing && (
        <EditDtb
          key={editing.name}
          dtb={editing}
          close={() => setEditing(null)}
        />
      )}
    </>
  );
}
function EditDtb({ dtb, close }: { dtb: DtbFileResponse; close: () => void }) {
  const [name, setName] = useState(dtb.name);
  const file = useRef<File | null>(null);
  const task = useAction();
  return (
    <Dialog
      open
      onOpenChange={(v) => {
        if (!v && !task.pending) close();
      }}
    >
      <DialogContent>
        <DialogTitle>修改 DTB</DialogTitle>
        <DialogDescription>
          可重命名或替换文件，单个文件最大 10 MiB。
        </DialogDescription>
        <FieldGroup>
          <TextField label="文件名" value={name} onValue={setName} />
          <TextField
            label="替换文件"
            type="file"
            accept=".dtb,application/octet-stream"
            onChange={(e) => {
              file.current = e.target.files?.[0] ?? null;
              if (file.current) setName(file.current.name);
            }}
          />
        </FieldGroup>
        {task.error && <Notice>{task.error}</Notice>}
        <DialogFooter>
          <Button variant="outline" onClick={close} disabled={task.pending}>
            取消
          </Button>
          <Button
            disabled={task.pending}
            onClick={() =>
              void task.run(async () => {
                validateFile(file.current);
                if (!name.trim()) throw new Error("文件名不能为空");
                if (name === dtb.name && !file.current)
                  throw new Error("请修改文件名或选择替换文件");
                await api.updateDtb(dtb.name, name, file.current);
                close();
              }, "DTB 已更新")
            }
          >
            保存修改
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
function formatSize(n: number) {
  return n < 1024
    ? `${n} B`
    : n < 1024 * 1024
      ? `${(n / 1024).toFixed(1)} KiB`
      : `${(n / 1024 / 1024).toFixed(1)} MiB`;
}

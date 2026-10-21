import { useState } from "react";
import { api } from "@/api/client";
import { useList } from "@/api/events";
import type { Rental, RentalInput } from "@/types/api";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { ListToolbar } from "@/components/list-toolbar";
import {
  TextField,
  SelectField,
  DataTable,
  Row,
  Cell,
  Notice,
  ConfirmAction,
  useAction,
} from "@/components/forms";

export const rentalStatusLabels = {
  scheduled: "未开始",
  active: "使用期内",
  expired: "已结束",
  cancelled: "已取消",
};
function localInput(iso: string) {
  const date = new Date(iso);
  return new Date(date.getTime() - date.getTimezoneOffset() * 60000)
    .toISOString()
    .slice(0, 23);
}
export default function Rentals() {
  const rentals = useList("rentals"),
    users = useList("users"),
    boards = useList("boards");
  const [status, setStatus] = useState("");
  const [user, setUser] = useState("");
  const [board, setBoard] = useState("");
  const [search, setSearch] = useState("");
  const [editor, setEditor] = useState<{
    id?: string;
    draft: RentalInput;
  } | null>(null);
  const task = useAction();
  function open(rental?: Rental) {
    setEditor({
      id: rental?.id,
      draft: {
        board_id: rental?.board_id ?? boards.find((b) => !b.disabled)?.id ?? "",
        user_id: rental?.user_id ?? users.find((u) => !u.disabled)?.id ?? "",
        starts_at: localInput(rental?.starts_at ?? new Date().toISOString()),
        ends_at: localInput(
          rental?.ends_at ?? new Date(Date.now() + 3600000).toISOString(),
        ),
        notes: rental?.notes ?? "",
      },
    });
  }
  const patch = (change: Partial<RentalInput>) =>
    setEditor((e) => (e ? { ...e, draft: { ...e.draft, ...change } } : e));
  const filtered = rentals.filter(
    (r) =>
      (!status || r.status === status) &&
      (!user || r.user_id === user) &&
      (!board || r.board_id === board) &&
      `${r.board_id} ${users.find((u) => u.id === r.user_id)?.email ?? ""} ${r.notes}`
        .toLowerCase()
        .includes(search.toLowerCase()),
  );
  return (
    <>
      <ListToolbar
        actions={
          <Button
            onClick={() => open()}
            disabled={
              !boards.some((b) => !b.disabled) ||
              !users.some((u) => !u.disabled)
            }
          >
            新增租赁
          </Button>
        }
        count={`${filtered.length} 条`}
      >
        <TextField
          label="搜索租赁"
          value={search}
          onValue={setSearch}
          placeholder="开发板、用户邮箱或备注"
        />
        <SelectField
          label="用户筛选"
          value={user}
          onValue={setUser}
          options={[
            { value: "", label: "全部用户" },
            ...users.map((u) => ({
              value: u.id,
              label: u.display_name || u.email,
            })),
          ]}
        />
        <SelectField
          label="开发板筛选"
          value={board}
          onValue={setBoard}
          options={[
            { value: "", label: "全部开发板" },
            ...boards.map((b) => ({ value: b.id, label: b.id })),
          ]}
        />
        <SelectField
          label="租赁状态"
          value={status}
          onValue={setStatus}
          options={[
            { value: "", label: "全部状态" },
            ...Object.entries(rentalStatusLabels).map(([value, label]) => ({
              value,
              label,
            })),
          ]}
        />
        {(search || user || board || status) && (
          <Button
            variant="ghost"
            onClick={() => {
              setSearch("");
              setUser("");
              setBoard("");
              setStatus("");
            }}
          >
            重置
          </Button>
        )}
      </ListToolbar>
      <DataTable
        headers={[
          "开发板",
          "使用用户",
          "开始时间",
          "结束时间",
          "状态",
          "备注",
          "操作",
        ]}
        empty={!filtered.length}
      >
        {filtered.map((r) => (
          <Row key={r.id}>
            <Cell>
              <strong>{r.board_id}</strong>
            </Cell>
            <Cell>
              {users.find((u) => u.id === r.user_id)?.display_name ||
                users.find((u) => u.id === r.user_id)?.email ||
                r.user_id}
            </Cell>
            <Cell>{new Date(r.starts_at).toLocaleString("zh-CN")}</Cell>
            <Cell>{new Date(r.ends_at).toLocaleString("zh-CN")}</Cell>
            <Cell>{rentalStatusLabels[r.status]}</Cell>
            <Cell>{r.notes || "—"}</Cell>
            <Cell>
              <div className="actions">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={r.status === "expired" || r.status === "cancelled"}
                  onClick={() => open(r)}
                >
                  编辑
                </Button>
                {r.status !== "cancelled" && r.status !== "expired" && (
                  <ConfirmAction
                    label="取消租赁"
                    description={`取消 ${r.board_id} 的租赁？相关会话会终止，并按正常流程断电、清理文件。`}
                    action={() => api.cancelRental(r.id)}
                  />
                )}
              </div>
            </Cell>
          </Row>
        ))}
      </DataTable>
      <Dialog
        open={!!editor}
        onOpenChange={(value) => {
          if (!value && !task.pending) setEditor(null);
        }}
      >
        <DialogContent>
          <DialogTitle>{editor?.id ? "编辑租赁" : "新增租赁"}</DialogTitle>
          <DialogDescription>
            时间按当前浏览器时区输入，保存为
            UTC。时间段包含开始、不包含结束；管理员仍可执行维护操作。
          </DialogDescription>
          {editor && (
            <form
              className="management-form"
              onSubmit={(event) => {
                event.preventDefault();
                void task
                  .run(async () => {
                    const payload = {
                      ...editor.draft,
                      starts_at: new Date(editor.draft.starts_at).toISOString(),
                      ends_at: new Date(editor.draft.ends_at).toISOString(),
                    };
                    if (
                      new Date(payload.starts_at) >= new Date(payload.ends_at)
                    )
                      throw new Error("开始时间必须早于结束时间");
                    if (editor.id) await api.updateRental(editor.id, payload);
                    else await api.createRental(payload);
                  }, "租赁已保存")
                  .then((ok) => {
                    if (ok) setEditor(null);
                  });
              }}
            >
              <SelectField
                label="使用用户"
                disabled={!!editor.id}
                value={editor.draft.user_id}
                onValue={(user_id) => patch({ user_id })}
                options={users
                  .filter((u) => !u.disabled || u.id === editor.draft.user_id)
                  .map((u) => ({
                    value: u.id,
                    label: `${u.display_name || u.email} · ${u.email}`,
                  }))}
              />
              <SelectField
                label="指定开发板"
                disabled={!!editor.id}
                value={editor.draft.board_id}
                onValue={(board_id) => patch({ board_id })}
                options={boards
                  .filter((b) => !b.disabled || b.id === editor.draft.board_id)
                  .map((b) => ({
                    value: b.id,
                    label: `${b.id} · ${b.board_type}`,
                  }))}
              />
              <TextField
                label="开始时间"
                type="datetime-local"
                step="0.001"
                required
                value={editor.draft.starts_at}
                onValue={(starts_at) => patch({ starts_at })}
              />
              <TextField
                label="结束时间"
                type="datetime-local"
                step="0.001"
                required
                value={editor.draft.ends_at}
                onValue={(ends_at) => patch({ ends_at })}
              />
              <TextField
                label="备注"
                maxLength={2000}
                value={editor.draft.notes}
                onValue={(notes) => patch({ notes })}
              />
              {task.error && <Notice>{task.error}</Notice>}
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  disabled={task.pending}
                  onClick={() => setEditor(null)}
                >
                  返回
                </Button>
                <Button disabled={task.pending} type="submit">
                  {task.pending ? "保存中…" : "保存租赁"}
                </Button>
              </DialogFooter>
            </form>
          )}
        </DialogContent>
      </Dialog>
    </>
  );
}

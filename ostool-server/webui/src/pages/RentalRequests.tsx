import { useState } from "react";
import { api } from "@/api/client";
import { useList } from "@/api/events";
import type { RentalRequest } from "@/types/api";
import { PagePanel } from "@/components/page-panel";
import { ListToolbar } from "@/components/list-toolbar";
import { useListPage } from "@/components/list-pagination";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  TextField,
  TextareaField,
  SelectField,
  Notice,
  DataTable,
  Row,
  Cell,
  useAction,
} from "@/components/forms";
import { requestStatusLabels } from "./MemberWorkspace";
export default function RentalRequests() {
  const requests = useList("rental_requests"),
    [status, setStatus] = useState(""),
    [search, setSearch] = useState("");
  const [editor, setEditor] = useState<{
      request: RentalRequest;
      decision: "approve" | "reject";
    } | null>(null),
    [note, setNote] = useState("");
  const task = useAction();
  const filtered = requests.filter(
    (r) =>
      (!status || r.status === status) &&
      `${r.board_id} ${r.user_email} ${r.notes}`
        .toLowerCase()
        .includes(search.toLowerCase()),
  );
  const page = useListPage(filtered, JSON.stringify([search, status]));
  function open(request: RentalRequest, decision: "approve" | "reject") {
    setNote("");
    setEditor({ request, decision });
  }
  return (
    <>
      <PagePanel
        table
        toolbar={
          <ListToolbar
            actions={
              <span>
                待审核 {requests.filter((r) => r.status === "pending").length}{" "}
                条
              </span>
            }
            count={`${filtered.length} 条`}
          >
            <TextField
              label="搜索申请"
              placeholder="开发板、邮箱或说明"
              value={search}
              onValue={setSearch}
            />
            <SelectField
              label="申请状态"
              value={status}
              onValue={setStatus}
              options={[
                { value: "", label: "全部状态" },
                ...Object.entries(requestStatusLabels).map(
                  ([value, label]) => ({ value, label }),
                ),
              ]}
            />
          </ListToolbar>
        }
        footer={page.footer}
      >
        <DataTable
          headers={[
            "申请用户",
            "开发板 / 说明",
            "申请时段",
            "状态 / 审核意见",
            "操作",
          ]}
          empty={!filtered.length}
        >
          {page.rows.map((r) => (
            <Row key={r.id}>
              <Cell>{r.user_email}</Cell>
              <Cell>
                <strong>{r.board_id}</strong>
                <small className="block">{r.notes || "—"}</small>
              </Cell>
              <Cell>
                {new Date(r.starts_at).toLocaleString()}
                <br />
                {new Date(r.ends_at).toLocaleString()}
              </Cell>
              <Cell>
                {requestStatusLabels[r.status]}
                <small className="block">{r.decision_note || "—"}</small>
              </Cell>
              <Cell>
                {r.status === "pending" && (
                  <div className="row-actions">
                    <Button size="sm" onClick={() => open(r, "approve")}>
                      批准
                    </Button>
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => open(r, "reject")}
                    >
                      拒绝
                    </Button>
                  </div>
                )}
              </Cell>
            </Row>
          ))}
        </DataTable>
      </PagePanel>
      <Dialog
        open={!!editor}
        onOpenChange={(open) => {
          if (!open && !task.pending) setEditor(null);
        }}
      >
        <DialogContent>
          <DialogTitle>
            {editor?.decision === "approve" ? "批准租赁申请" : "拒绝租赁申请"}
          </DialogTitle>
          <DialogDescription>
            {editor?.request.user_email} · {editor?.request.board_id}
            。批准时重新检查时间冲突并创建正式租赁。
          </DialogDescription>
          {task.error && <Notice>{task.error}</Notice>}
          <TextareaField label="审核意见" value={note} onValue={setNote} />
          <DialogFooter>
            <Button
              variant="outline"
              disabled={task.pending}
              onClick={() => setEditor(null)}
            >
              取消
            </Button>
            <Button
              disabled={task.pending}
              onClick={async () => {
                if (
                  editor &&
                  (await task.run(
                    () =>
                      api.reviewRentalRequest(
                        editor.request.id,
                        editor.decision,
                        note,
                      ),
                    "申请已处理",
                  ))
                )
                  setEditor(null);
              }}
            >
              确认审核
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

import { Pencil as ActionEdit, CircleX as ActionCancel } from "lucide-react";
import { RowAction } from "@/components/row-action";
import { PagePanel } from "@/components/page-panel";
import { useListPage } from "@/components/list-pagination";
import { useState } from "react";
import { Link } from "react-router-dom";
import { api } from "@/api/client";
import { useList } from "@/api/events";
import { Button } from "@/components/ui/button";
import { ListToolbar } from "@/components/list-toolbar";
import {
  TextField,
  SelectField,
  DataTable,
  Row,
  Cell,
  ConfirmAction,
} from "@/components/forms";

export const rentalStatusLabels = {
  scheduled: "未开始",
  active: "使用期内",
  expired: "已结束",
  cancelled: "已取消",
};
export default function Rentals() {
  const rentals = useList("rentals"),
    users = useList("users"),
    boards = useList("boards");
  const [status, setStatus] = useState("");
  const [user, setUser] = useState("");
  const [board, setBoard] = useState("");
  const [search, setSearch] = useState("");
  const filtered = rentals.filter(
    (r) =>
      (!status || r.status === status) &&
      (!user || r.user_id === user) &&
      (!board || r.board_id === board) &&
      `${r.board_id} ${users.find((u) => u.id === r.user_id)?.email ?? ""} ${r.notes}`
        .toLowerCase()
        .includes(search.toLowerCase()),
  );
  const pagination = useListPage(
    filtered,
    JSON.stringify([search, user, board, status]),
  );
  return (
    <>
      <PagePanel
        table
        toolbar={
          <ListToolbar
            actions={
              <Button
                asChild
                disabled={
                  !boards.some((b) => !b.disabled) ||
                  !users.some((u) => !u.disabled)
                }
              >
                <Link to="/rentals/new">新增租赁</Link>
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
        }
        footer={pagination.footer}
        scrollKey={pagination.viewKey}
      >
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
          {pagination.rows.map((r) => (
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
                  <RowAction
                    label="编辑"
                    icon={ActionEdit}
                    disabled={
                      r.status === "expired" || r.status === "cancelled"
                    }
                    to={`/rentals/${r.id}`}
                  />
                  {r.status !== "cancelled" && r.status !== "expired" && (
                    <ConfirmAction
                      icon={ActionCancel}
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
      </PagePanel>
    </>
  );
}

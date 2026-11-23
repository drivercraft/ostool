import { useSearchParams } from "react-router-dom";
import { Copy, CircuitBoard, Terminal } from "lucide-react";
import { ListActions } from "@/components/list-actions";
import { MessageToast } from "@/components/message-toast";
import { Badge } from "@/components/ui/badge";
import { Pencil as ActionEdit, CircleX as ActionCancel } from "lucide-react";
import { RowAction } from "@/components/row-action";
import { PagePanel } from "@/components/page-panel";
import { useServerList } from "@/components/server-list";
import { useEffect, useState } from "react";
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
  const [filters] = useSearchParams();
  const boards = useList("boards");
  const [status, setStatus] = useState("");
  const [user, setUser] = useState(filters.get("user") ?? "");
  useEffect(() => setUser(filters.get("user") ?? ""), [filters]);
  const [board, setBoard] = useState(filters.get("board") ?? "");
  useEffect(() => setBoard(filters.get("board") ?? ""), [filters]);
  const [search, setSearch] = useState("");
  const pagination = useServerList("rentals", {
    q: search,
    user,
    board,
    status,
  });
  return (
    <>
      {pagination.error && <MessageToast message={pagination.error} />}
      <PagePanel
        table
        loading={pagination.loading}
        updating={pagination.updating}
        toolbar={
          <ListToolbar
            actions={
              <Button
                permission="rentals.manage"
                asChild
                disabled={!boards.some((b) => !b.disabled)}
              >
                <Link to="/rentals/new">新增租赁</Link>
              </Button>
            }
            count={`${pagination.total} 条`}
          >
            <TextField
              label="搜索租赁"
              value={search}
              onValue={setSearch}
              placeholder="开发板、用户邮箱或备注"
            />
            <TextField
              label="用户筛选"
              value={user}
              onValue={setUser}
              placeholder="用户名、邮箱或用户 ID"
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
          rowOffset={pagination.rowOffset}
          headers={[
            "开发板",
            "使用用户",
            "开始时间",
            "结束时间",
            "状态",
            "备注",
            "操作",
          ]}
          empty={!pagination.rows.length}
        >
          {pagination.rows.map((r) => (
            <Row key={r.id}>
              <Cell>
                <strong>{r.board_id}</strong>
                <small>
                  {r.source === "instant" ? "即时使用" : "预约租赁"}
                </small>
              </Cell>
              <Cell>
                {r.user_display_name ||
                  r.user_username ||
                  r.user_email ||
                  r.user_id}
              </Cell>
              <Cell>{new Date(r.starts_at).toLocaleString("zh-CN")}</Cell>
              <Cell>{new Date(r.ends_at).toLocaleString("zh-CN")}</Cell>
              <Cell>
                <Badge
                  variant={
                    r.status === "active"
                      ? "success"
                      : r.status === "scheduled"
                        ? "warning"
                        : "secondary"
                  }
                >
                  {rentalStatusLabels[r.status]}
                </Badge>
              </Cell>
              <Cell>{r.notes || "—"}</Cell>
              <Cell>
                <ListActions
                  primary={[
                    <RowAction
                      key="edit"
                      label="编辑"
                      permission="rentals.manage"
                      icon={ActionEdit}
                      disabled={
                        r.source === "instant" ||
                        r.status === "expired" ||
                        r.status === "cancelled"
                      }
                      title={
                        r.source === "instant"
                          ? "即时使用租赁不支持编辑"
                          : r.status === "expired" || r.status === "cancelled"
                            ? "租赁已结束，不能编辑"
                            : "编辑"
                      }
                      to={`/rentals/${r.id}`}
                    />,
                    <ConfirmAction
                      key="cancel"
                      permission="rentals.manage"
                      icon={ActionCancel}
                      label="取消租赁"
                      disabled={
                        r.status === "cancelled" || r.status === "expired"
                      }
                      description={`取消 ${r.board_id} 的租赁？相关会话会终止，并按正常流程断电、清理文件。`}
                      action={() => api.cancelRental(r.id)}
                    />,
                  ]}
                >
                  <RowAction
                    key="基于此新建"
                    label="基于此新建"
                    icon={Copy}
                    permission="rentals.manage"
                    to={`/rentals/new?board=${encodeURIComponent(r.board_id)}&user=${encodeURIComponent(r.user_id)}`}
                  />
                  <RowAction
                    key="转到开发板"
                    label="转到开发板"
                    icon={CircuitBoard}
                    permission="devices.manage"
                    to={`/boards/${encodeURIComponent(r.board_id)}`}
                  />
                  <RowAction
                    key="转到会话"
                    label="转到会话"
                    icon={Terminal}
                    permission="sessions.read"
                    to={`/sessions?board=${encodeURIComponent(r.board_id)}`}
                  />
                </ListActions>
              </Cell>
            </Row>
          ))}
        </DataTable>
      </PagePanel>
    </>
  );
}

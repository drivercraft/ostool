import { RowAction } from "@/components/row-action";
import { CircuitBoard, Calendar } from "lucide-react";
import { ListActions } from "@/components/list-actions";
import { Unplug as ActionRelease } from "lucide-react";
import { ForceReleaseAction } from "@/components/force-release-action";
import { PagePanel } from "@/components/page-panel";
import { useListPage } from "@/components/list-pagination";
import { ListToolbar } from "@/components/list-toolbar";
import { useEffect, useState } from "react";
import { formatLeaseRemaining } from "@/utils/time";
import { useList, useResource } from "@/api/events";
import { api } from "@/api/client";
import { useEdition } from "@/edition";
import {
  DataTable,
  Row,
  Cell,
  ConfirmAction,
  TextField,
  SelectField,
} from "@/components/forms";
import {
  SessionStatus,
  sessionPresentation,
} from "@/apps/admin/components/session-status";
import { Link, Navigate, useSearchParams } from "react-router-dom";
export default function Sessions() {
  const [params] = useSearchParams();
  return params.get("view") === "history" ? (
    <Navigate replace to="/session-history" />
  ) : (
    <ActiveSessions />
  );
}
function ActiveSessions() {
  const publicEdition = useEdition().enabled;
  const [filters] = useSearchParams();
  const sessions = useList("sessions"),
    boards = useList("boards"),
    runtimes = useResource("runtimes") ?? {};
  const [search, setSearch] = useState("");
  const [board, setBoard] = useState(filters.get("board") ?? "");
  useEffect(() => setBoard(filters.get("board") ?? ""), [filters]);
  const [status, setStatus] = useState("");
  const filtered = sessions.filter(
    (s) =>
      (!board || s.board_id === board) &&
      (!status ||
        sessionPresentation(s, runtimes[s.board_id]).state === status) &&
      `${s.id} ${s.board_id} ${s.owner_username ?? ""} ${s.client_name ?? ""}`
        .toLowerCase()
        .includes(search.toLowerCase()),
  );
  const pagination = useListPage(
    filtered,
    JSON.stringify([search, board, status]),
  );
  return (
    <>
      <PagePanel
        table
        toolbar={
          <ListToolbar count={`${filtered.length} 个会话`}>
            <TextField
              label="搜索会话"
              value={search}
              onValue={setSearch}
              placeholder={
                publicEdition
                  ? "会话、设备、用户名或客户端"
                  : "会话、设备或客户端"
              }
            />
            <SelectField
              label="开发板筛选"
              value={board}
              onValue={setBoard}
              options={[
                { value: "", label: "全部开发板" },
                ...Array.from(new Set(sessions.map((s) => s.board_id))).map(
                  (id) => ({ value: id, label: id }),
                ),
              ]}
            />
            <SelectField
              label="会话状态"
              value={status}
              onValue={setStatus}
              options={[
                { value: "", label: "全部状态" },
                { value: "active", label: "活动会话" },
                { value: "releasing", label: "释放中" },
                { value: "release_failed", label: "释放失败" },
              ]}
            />
          </ListToolbar>
        }
        footer={pagination.footer}
        scrollKey={pagination.viewKey}
      >
        <DataTable
          rowOffset={pagination.rowOffset}
          headers={[
            "会话",
            "开发板",
            ...(publicEdition ? ["使用用户"] : []),
            "客户端",
            "创建 / 到期",
            "状态",
            "操作",
          ]}
          empty={!filtered.length}
        >
          {pagination.rows.map((s) => (
            <Row key={s.id}>
              <Cell className="mono">{s.id}</Cell>
              <Cell>
                <Link
                  className="text-link"
                  to={`/boards/${encodeURIComponent(s.board_id)}`}
                >
                  {s.board_id}
                </Link>
              </Cell>
              {publicEdition && (
                <Cell>
                  {s.owner_username ?? "本地会话"}
                  {s.rental_id && (
                    <small>
                      <Link to="/rentals">租赁 {s.rental_id.slice(0, 8)}</Link>
                    </small>
                  )}
                  {s.rental_ends_at && (
                    <small>
                      使用至{" "}
                      {new Date(s.rental_ends_at).toLocaleString("zh-CN")}
                    </small>
                  )}
                </Cell>
              )}
              <Cell>{s.client_name ?? "—"}</Cell>
              <Cell>
                <LeaseClock expires={s.expires_at} />
                <small>
                  创建 {new Date(s.created_at).toLocaleString("zh-CN")}
                </small>
                <small>
                  到期 {new Date(s.expires_at).toLocaleString("zh-CN")}
                </small>
              </Cell>
              <Cell>
                <SessionStatus
                  session={s}
                  runtime={runtimes[s.board_id]}
                  automaticSerial={
                    boards.find((board) => board.id === s.board_id)?.boot
                      .kind === "httpboot"
                  }
                />
              </Cell>
              <Cell>
                <ListActions
                  primary={[
                    <ConfirmAction
                      permission="sessions.manage"
                      icon={ActionRelease}
                      label="释放会话"
                      description={`请求释放 ${s.board_id} 的会话？服务器会停止任务并断电，清理完成后资源才重新可用。`}
                      action={() => api.deleteSession(s.id)}
                      disabled={
                        sessionPresentation(s, runtimes[s.board_id]).releasing
                      }
                    />,
                    <ForceReleaseAction
                      key="force-release"
                      sessionId={s.id}
                      boardId={s.board_id}
                    />,
                  ]}
                >
                  <RowAction
                    key="转到开发板"
                    label="转到开发板"
                    icon={CircuitBoard}
                    permission="devices.manage"
                    to={`/boards/${encodeURIComponent(s.board_id)}`}
                  />
                  <RowAction
                    key="转到租赁"
                    label="转到租赁"
                    icon={Calendar}
                    permission="rentals.read"
                    to={`/rentals?board=${encodeURIComponent(s.board_id)}`}
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

function LeaseClock({ expires }: { expires: string }) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  // Local clock rendering only; this never fetches or changes the lease.
  const remaining = formatLeaseRemaining(expires, new Date(now));
  return (
    <span>{remaining === "已过期" ? remaining : `剩余 ${remaining}`}</span>
  );
}

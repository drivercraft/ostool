import { Unplug as ActionRelease } from "lucide-react";
import { PagePanel } from "@/components/page-panel";
import { useListPage } from "@/components/list-pagination";
import { ListToolbar } from "@/components/list-toolbar";
import { Button } from "@/components/ui/button";
import { useEffect, useState } from "react";
import { formatLeaseRemaining } from "@/utils/time";
import { useList, useResource } from "@/api/events";
import { api } from "@/api/client";
import {
  DataTable,
  Row,
  Cell,
  ConfirmAction,
  TextField,
  SelectField,
} from "@/components/forms";
import { SerialRuntimeDetails } from "@/apps/admin/components/serial-runtime";
import { Badge } from "@/components/ui/badge";
import { Link, useSearchParams } from "react-router-dom";
import LeaseHistory from "./LeaseHistory";
export default function Sessions() {
  const [params] = useSearchParams();
  return params.get("view") === "history" ? (
    <LeaseHistory />
  ) : (
    <ActiveSessions />
  );
}
function ActiveSessions() {
  const sessions = useList("sessions"),
    runtimes = useResource("runtimes") ?? {};
  const [search, setSearch] = useState("");
  const [board, setBoard] = useState("");
  const [status, setStatus] = useState("");
  const filtered = sessions.filter(
    (s) =>
      (!board || s.board_id === board) &&
      (!status || s.state === status) &&
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
          <ListToolbar
            actions={
              <Button asChild variant="outline">
                <Link to="/sessions?view=history">租约历史</Link>
              </Button>
            }
            count={`${filtered.length} 个会话`}
          >
            <TextField
              label="搜索会话"
              value={search}
              onValue={setSearch}
              placeholder="会话、设备、用户名或客户端"
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
                { value: "active", label: "占用中" },
                { value: "releasing", label: "释放中" },
              ]}
            />
          </ListToolbar>
        }
        footer={pagination.footer}
        scrollKey={pagination.viewKey}
      >
        <DataTable
          headers={[
            "Session",
            "开发板",
            "使用用户",
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
              <Cell>
                {s.owner_username ?? "本地会话"}
                {s.rental_id && (
                  <small>
                    <Link to="/rentals">租赁 {s.rental_id.slice(0, 8)}</Link>
                  </small>
                )}
                {s.rental_ends_at && (
                  <small>
                    使用至 {new Date(s.rental_ends_at).toLocaleString("zh-CN")}
                  </small>
                )}
              </Cell>
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
                <Badge
                  variant={
                    runtimes[s.board_id]?.last_release_error
                      ? "destructive"
                      : "secondary"
                  }
                >
                  {runtimes[s.board_id]?.last_release_error
                    ? "释放失败，等待重试"
                    : s.state === "releasing"
                      ? "释放中"
                      : "占用中"}
                </Badge>
                <small>
                  WebSocket {s.serial_connected ? "已连接" : "未连接"}
                </small>
                <SerialRuntimeDetails status={s.serial_runtime} />
                <small className="text-destructive">
                  {runtimes[s.board_id]?.last_release_error}
                </small>
              </Cell>
              <Cell>
                <ConfirmAction
                  icon={ActionRelease}
                  label="强制释放"
                  description={`请求释放 ${s.board_id} 的会话？服务器会停止任务并断电，清理完成后资源才重新可用。`}
                  action={() => api.deleteSession(s.id)}
                  disabled={s.state === "releasing"}
                />
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
  return <span>{`剩余 ${formatLeaseRemaining(expires, new Date(now))}`}</span>;
}

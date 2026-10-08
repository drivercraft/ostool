import { useEffect, useState } from "react";
import { formatLeaseRemaining } from "@/utils/time";
import { useList, useResource } from "@/api/events";
import { api } from "@/api/client";
import { DataTable, Row, Cell, ConfirmAction } from "@/components/forms";
import { SerialRuntimeDetails } from "@/components/serial-runtime";
import { Badge } from "@/components/ui/badge";
import { Link } from "react-router-dom";
export default function Sessions() {
  const sessions = useList("sessions"),
    runtimes = useResource("runtimes") ?? {};
  return (
    <>
      <header className="page-header">
        <h1>会话租约</h1>
      </header>
      <p className="page-description">租约释放及失败恢复状态实时同步。</p>
      <DataTable
        headers={["Session", "开发板", "客户端", "创建 / 到期", "状态", "操作"]}
        empty={!sessions.length}
      >
        {sessions.map((s) => (
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
                label="强制释放"
                description={`请求释放 ${s.board_id} 的会话？服务器会停止任务并断电，清理完成后资源才重新可用。`}
                action={() => api.deleteSession(s.id)}
                disabled={s.state === "releasing"}
              />
            </Cell>
          </Row>
        ))}
      </DataTable>
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

import { ListPagination } from "@/components/list-pagination";
import { MessageToast } from "@/components/message-toast";
import { useEffect, useMemo, useRef, useState } from "react";
import { Eye } from "lucide-react";
import { useEdition } from "@/edition";
import { admin, useErrors, useResource } from "@/api/events";
import type { LeaseFilters, LeaseRecord } from "@/types/api";
import { PagePanel } from "@/components/page-panel";
import { ListToolbar } from "@/components/list-toolbar";
import { RowAction } from "@/components/row-action";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogBody,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import {
  DataTable,
  Row,
  Cell,
  TextField,
  SelectField,
  ReadonlyInfo,
  ListDataLoadingContext,
} from "@/components/forms";

const reasons: Record<string, string> = {
  client_released: "客户端释放",
  admin_released: "管理员释放",
  admin_forced: "管理员强制结束",
  serial_closed: "串口连接关闭",
  boot_failed: "启动失败",
  lease_expired: "租约到期 / 心跳超时",
  rental_revoked: "租赁取消或权限失效",
  authorization_unavailable: "鉴权或租约存储不可用",
  server_shutdown: "服务器正常关闭",
  server_interrupted: "服务中断后恢复",
  session_dropped: "会话被回收",
};
const date = (value: string | null) =>
  value ? new Date(value).toLocaleString("zh-CN", { hour12: false }) : "—";
const key = (filters: LeaseFilters) =>
  JSON.stringify(
    Object.entries(filters)
      .filter(([, value]) => value != null && value !== "")
      .map(([name, value]) => [
        name,
        name === "from" || name === "to" ? Date.parse(String(value)) : value,
      ])
      .sort(([a], [b]) => String(a).localeCompare(String(b))),
  );
function duration(record: LeaseRecord) {
  if (!record.ended_at) return "—";
  const seconds = Math.max(
    0,
    Math.floor(
      (Date.parse(record.ended_at) - Date.parse(record.created_at)) / 1000,
    ),
  );
  return `${Math.floor(seconds / 3600)} 小时 ${Math.floor(seconds / 60) % 60} 分 ${seconds % 60} 秒`;
}
export default function LeaseHistory() {
  const unknownOwner = !useEdition().enabled ? "本地操作者" : "未完成身份绑定";
  const [filters, setFilters] = useState<LeaseFilters>({
    page: 1,
    page_size: 20,
  });
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [details, setDetails] = useState<LeaseRecord | null>(null);
  const data = useResource("session_history");
  const error = useErrors().session_history;
  const previousSearch = useRef<string | undefined>(undefined);
  const identity = useMemo(() => key(filters), [filters]);
  const filterError =
    from && to && Date.parse(from) >= Date.parse(to)
      ? "起始时间必须早于截止时间"
      : "";
  useEffect(() => {
    if (filterError) return;
    const delayed =
      previousSearch.current !== undefined &&
      previousSearch.current !== filters.q;
    previousSearch.current = filters.q;
    if (!delayed) {
      admin.setLeaseFilters(filters);
      return;
    }
    const timer = setTimeout(() => admin.setLeaseFilters(filters), 300);
    return () => clearTimeout(timer);
  }, [identity, filterError]);
  useEffect(() => () => admin.setLeaseFilters(null), []);
  const ready = !!data?.active && key(data.query ?? {}) === identity;
  const rows = data?.active ? (data.items ?? []) : [];
  const total = data?.total ?? 0,
    page = data?.page ?? 1,
    size = data?.page_size ?? filters.page_size ?? 20;
  function change(value: Partial<LeaseFilters>) {
    setFilters((old) => ({ ...old, ...value, page: value.page ?? 1 }));
  }
  const footer = (
    <ListPagination
      total={total}
      page={page}
      size={size}
      pending={!ready || !!filterError}
      onPage={(page) => change({ page })}
      onSize={(page_size) => change({ page_size })}
    />
  );
  return (
    <>
      <PagePanel
        table
        updating={!ready && !!data?.active && !error}
        layoutKey="history"
        scrollKey={identity}
        footer={footer}
        toolbar={
          <ListToolbar count={ready ? `${total} 条历史记录` : "正在同步…"}>
            <TextField
              label="搜索会话历史"
              placeholder="会话、设备、用户名或客户端"
              value={filters.q ?? ""}
              onValue={(q) => change({ q })}
            />
            <TextField
              label="设备标识"
              placeholder="按完整设备 ID 筛选"
              value={filters.board ?? ""}
              onValue={(board) => change({ board })}
            />
            <SelectField
              label="会话状态"
              value={filters.state ?? ""}
              onValue={(state) => change({ state })}
              options={[
                { value: "", label: "全部状态" },
                { value: "active", label: "使用中" },
                { value: "releasing", label: "释放中" },
                { value: "ended", label: "已结束" },
              ]}
            />
            <SelectField
              label="结束原因"
              value={filters.reason ?? ""}
              onValue={(reason) => change({ reason })}
              options={[
                { value: "", label: "全部结束原因" },
                ...Object.entries(reasons).map(([value, label]) => ({
                  value,
                  label,
                })),
              ]}
            />
            <TextField
              label="开始时间从"
              type="datetime-local"
              value={from}
              onValue={(value) => {
                setFrom(value);
                change({
                  from: value ? new Date(value).toISOString() : undefined,
                });
              }}
            />
            <TextField
              label="开始时间至"
              type="datetime-local"
              value={to}
              onValue={(value) => {
                setTo(value);
                change({
                  to: value ? new Date(value).toISOString() : undefined,
                });
              }}
            />
          </ListToolbar>
        }
      >
        {filterError && <MessageToast message={filterError} />}
        {error && <MessageToast message={error} />}
        <ListDataLoadingContext.Provider
          value={!data?.active && !error && !filterError}
        >
          <DataTable
            rowOffset={(page - 1) * size}
            headers={[
              "会话 / 设备",
              "使用用户",
              "客户端",
              "起止时间",
              "使用时长",
              "状态 / 结束原因",
              "操作",
            ]}
            empty={!rows.length}
          >
            {rows.map((record) => (
              <Row key={record.session_id}>
                <Cell>
                  <span className="mono">{record.session_id}</span>
                  <small>
                    {record.board_id} · {record.board_type}
                  </small>
                </Cell>
                <Cell>
                  {record.username ?? unknownOwner}
                  {record.rental_id && (
                    <small>租赁 {record.rental_id.slice(0, 8)}</small>
                  )}
                </Cell>
                <Cell>{record.client_name ?? "—"}</Cell>
                <Cell>
                  {date(record.created_at)}
                  <small>
                    至 {record.ended_at ? date(record.ended_at) : "尚未结束"}
                  </small>
                </Cell>
                <Cell>
                  {duration(record)}
                  {record.end_reason === "server_interrupted" && (
                    <small>结束时间为中断发现时间</small>
                  )}
                </Cell>
                <Cell>
                  <Badge
                    variant={
                      record.last_release_error && !record.ended_at
                        ? "destructive"
                        : record.ended_at
                          ? "secondary"
                          : record.stop_requested_at
                            ? "warning"
                            : "info"
                    }
                  >
                    {record.ended_at
                      ? "已结束"
                      : record.stop_requested_at
                        ? "释放中"
                        : "使用中"}
                  </Badge>
                  <small>
                    {record.end_reason
                      ? (reasons[record.end_reason] ?? record.end_reason)
                      : "—"}
                  </small>
                  {record.release_failures > 0 && (
                    <small>曾重试释放 {record.release_failures} 次</small>
                  )}
                </Cell>
                <Cell>
                  <RowAction
                    label="会话详情"
                    icon={Eye}
                    onClick={() => setDetails(record)}
                  />
                </Cell>
              </Row>
            ))}
          </DataTable>
        </ListDataLoadingContext.Provider>
      </PagePanel>
      <Dialog
        open={!!details}
        onOpenChange={(open) => {
          if (!open) setDetails(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>会话详情</DialogTitle>
          </DialogHeader>
          <DialogBody>
            <DialogDescription>
              记录保留分配时的设备与用户名。中断记录的结束时间表示重启时发现连接已失效的时间。
            </DialogDescription>
            {details && (
              <ReadonlyInfo
                entries={{
                  会话: details.session_id,
                  设备: details.board_id,
                  板型: details.board_type,
                  用户名: details.username ?? unknownOwner,
                  用户ID: details.user_id ?? "—",
                  租赁ID: details.rental_id ?? "—",
                  客户端: details.client_name ?? "—",
                  创建时间: date(details.created_at),
                  最后心跳: date(details.last_heartbeat_at),
                  最后租约到期: date(details.expires_at),
                  请求释放时间: date(details.stop_requested_at),
                  结束时间: date(details.ended_at),
                  使用时长: duration(details),
                  释放原因: details.stop_reason
                    ? (reasons[details.stop_reason] ?? details.stop_reason)
                    : "—",
                  结束原因: details.end_reason
                    ? (reasons[details.end_reason] ?? details.end_reason)
                    : "—",
                  释放失败次数: details.release_failures,
                  最后释放错误: details.last_release_error ?? "—",
                }}
              />
            )}
          </DialogBody>
        </DialogContent>
      </Dialog>
    </>
  );
}

import { RowAction } from "@/components/row-action";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  Download,
  Eye,
  Filter,
  Trash2,
} from "lucide-react";
import { admin, useErrors, useResource } from "@/api/events";
import { api } from "@/api/client";
import { PagePanel } from "@/components/page-panel";
import { ListToolbar } from "@/components/list-toolbar";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  DataTable,
  Row,
  Cell,
  TextField,
  SelectField,
  ReadonlyInfo,
  Notice,
  useAction,
} from "@/components/forms";
import type { AuditEvent, AuditFilters } from "@/types/api";
const resources: Record<string, string> = {
  boards: "开发板",
  sessions: "会话",
  rentals: "租赁",
  "rental-requests": "租赁申请",
  users: "用户",
  roles: "角色",
  dtbs: "DTB",
  auth: "账号认证",
  profile: "个人资料",
  password: "密码",
  audits: "审计日志",
  system: "系统",
  "server-config": "系统设置",
  tftp: "TFTP",
  "power-actions": "电源操作",
  "loader-images": "OTA 镜像",
  ota: "OTA 升级",
  "virtual-devices": "虚拟设备",
  token: "登录凭据",
  revoke: "退出登录",
};
const sources: Record<string, string> = {
  admin: "管理后台",
  website: "网站",
  cli: "CLI / API",
  system: "系统任务",
};
const results: Record<string, string> = {
  success: "成功",
  accepted: "已受理",
  denied: "拒绝",
  failed: "失败",
  pending: "结果未确认",
};
const actors: Record<string, string> = {
  admin: "管理员",
  member: "成员",
  local: "本地操作者",
  anonymous: "未认证",
  system: "系统",
};
const date = (value: string | null) =>
  value ? new Date(value).toLocaleString("zh-CN", { hour12: false }) : "—";
const validIp = (value: string) => {
  if (value.includes(":")) {
    try {
      return new URL(`http://[${value}]/`).hostname.startsWith("[");
    } catch {
      return false;
    }
  }
  return (
    value.split(".").length === 4 &&
    value
      .split(".")
      .every(
        (part) => /^(0|[1-9][0-9]{0,2})$/.test(part) && Number(part) <= 255,
      )
  );
};
const key = (filters: AuditFilters) =>
  JSON.stringify(
    Object.entries(filters)
      .filter(([, value]) => value != null && value !== "")
      .map(([name, value]) => [
        name,
        name === "from" || name === "to" ? Date.parse(String(value)) : value,
      ])
      .sort(([a], [b]) => String(a).localeCompare(String(b))),
  );
export default function Audits() {
  const [filters, setFilters] = useState<AuditFilters>({
      page: 1,
      page_size: 20,
    }),
    [advanced, setAdvanced] = useState(false),
    [from, setFrom] = useState(""),
    [to, setTo] = useState("");
  const error = useErrors().audit;
  const previousSearch = useRef<string | undefined>(undefined);
  const data = useResource("audit"),
    viewed = useRef(false),
    [details, setDetails] = useState<AuditEvent | null>(null),
    [detailOpen, setDetailOpen] = useState(false),
    [cleanup, setCleanup] = useState(false),
    [days, setDays] = useState("180");
  const task = useAction();
  const filterError =
    from && to && Date.parse(from) >= Date.parse(to)
      ? "起始时间必须早于截止时间"
      : filters.ip && !validIp(filters.ip.trim())
        ? "请输入有效的 IPv4 或 IPv6 地址"
        : "";
  const identity = useMemo(() => key(filters), [filters]);
  useEffect(() => {
    if (filterError) return;
    const delayed =
      previousSearch.current !== undefined &&
      previousSearch.current !== filters.q;
    previousSearch.current = filters.q;
    if (!delayed) {
      admin.setAuditFilters(filters);
      return;
    }
    const timer = setTimeout(() => admin.setAuditFilters(filters), 300);
    return () => clearTimeout(timer);
  }, [identity, filterError]);
  useEffect(() => {
    if (!viewed.current) {
      viewed.current = true;
      void api.auditViewed().catch(() => {});
    }
    return () => admin.setAuditFilters(null);
  }, []);
  const ready = !!data?.active && key(data.query ?? {}) === identity,
    rows = data?.active ? (data?.items ?? []) : [],
    total = data?.total ?? 0,
    page = data?.page ?? 1,
    size = data?.page_size ?? filters.page_size ?? 20,
    pages = Math.max(1, Math.ceil(total / size));
  function change(value: Partial<AuditFilters>) {
    setFilters((old) => ({ ...old, ...value, page: value.page ?? 1 }));
  }
  async function open(id: string) {
    setDetails(null);
    setDetailOpen(true);
    await task.run(async () => setDetails(await api.auditDetail(id)));
  }
  async function download() {
    await task.run(async () => {
      const blob = await api.exportAudit(filters);
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = "ostool-audit.csv";
      link.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    }, "审计日志已导出");
  }
  const footer = (
    <div className="audit-pagination">
      <span>
        共 {total} 条 ·{" "}
        {ready
          ? `${total ? (page - 1) * size + 1 : 0}–${Math.min(page * size, total)}`
          : "同步中…"}
      </span>
      <div>
        <SelectField
          label="每页条数"
          value={String(filters.page_size)}
          onValue={(value) => change({ page_size: Number(value) })}
          options={[10, 20, 50, 100].map((value) => ({
            value: String(value),
            label: `${value} 条/页`,
          }))}
        />
        <Button
          size="icon-sm"
          variant="outline"
          aria-label="上一页"
          disabled={!ready || page <= 1}
          onClick={() => change({ page: page - 1 })}
        >
          <ChevronLeft />
        </Button>
        <span>
          第 {page} / {pages} 页
        </span>
        <Button
          size="icon-sm"
          variant="outline"
          aria-label="下一页"
          disabled={!ready || page >= pages}
          onClick={() => change({ page: page + 1 })}
        >
          <ChevronRight />
        </Button>
      </div>
    </div>
  );
  return (
    <>
      <PagePanel
        table
        loading={!data?.active && !error}
        updating={!ready && !!data?.active && !error}
        scrollKey={identity}
        toolbar={
          <>
            <ListToolbar
              actions={
                <>
                  <Button
                    variant="outline"
                    disabled={task.pending || !!filterError || !ready}
                    onClick={() => void download()}
                  >
                    <Download size={15} />
                    导出 CSV
                  </Button>
                  <Button variant="outline" onClick={() => setCleanup(true)}>
                    <Trash2 size={15} />
                    清理旧日志
                  </Button>
                </>
              }
              count={ready ? `${total} 条` : "正在同步…"}
            >
              <TextField
                label="搜索审计日志"
                placeholder="操作、操作者、对象、请求 ID"
                maxLength={256}
                value={filters.q ?? ""}
                onValue={(q) => change({ q })}
              />
              <SelectField
                label="操作类型"
                value={filters.action ?? ""}
                onValue={(action) => change({ action })}
                options={[
                  { value: "", label: "全部操作" },
                  ...(data?.actions ?? []).map((item) => ({
                    value: item.value,
                    label: item.label,
                  })),
                ]}
              />
              <SelectField
                label="资源类型"
                value={filters.resource ?? ""}
                onValue={(resource) => change({ resource })}
                options={[
                  { value: "", label: "全部资源" },
                  ...(data?.resources ?? []).map((value) => ({
                    value,
                    label: resources[value] ?? value,
                  })),
                ]}
              />
              <SelectField
                label="操作结果"
                value={filters.result ?? ""}
                onValue={(result) => change({ result })}
                options={[
                  { value: "", label: "全部结果" },
                  ...Object.entries(results).map(([value, label]) => ({
                    value,
                    label,
                  })),
                ]}
              />
              <Button
                variant="ghost"
                aria-expanded={advanced}
                onClick={() => setAdvanced((value) => !value)}
              >
                <Filter size={15} />
                更多筛选
              </Button>
            </ListToolbar>
            {advanced && (
              <div className="audit-advanced">
                <TextField
                  label="操作者"
                  placeholder="名称或邮箱"
                  maxLength={256}
                  value={filters.actor ?? ""}
                  onValue={(actor) => change({ actor })}
                />
                <TextField
                  label="来源 IP"
                  placeholder="IPv4 或 IPv6"
                  maxLength={45}
                  value={filters.ip ?? ""}
                  onValue={(ip) => change({ ip: ip.trim() })}
                />
                <SelectField
                  label="操作来源"
                  value={filters.source ?? ""}
                  onValue={(source) => change({ source })}
                  options={[
                    { value: "", label: "全部来源" },
                    ...Object.entries(sources).map(([value, label]) => ({
                      value,
                      label,
                    })),
                  ]}
                />
                <TextField
                  label="起始时间"
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
                  label="截止时间"
                  type="datetime-local"
                  value={to}
                  onValue={(value) => {
                    setTo(value);
                    change({
                      to: value ? new Date(value).toISOString() : undefined,
                    });
                  }}
                />
                <Button
                  variant="ghost"
                  onClick={() => {
                    setFilters({ page: 1, page_size: 20 });
                    setFrom("");
                    setTo("");
                  }}
                >
                  重置筛选
                </Button>
              </div>
            )}
          </>
        }
        footer={footer}
      >
        {filterError && <Notice>{filterError}</Notice>}
        {task.error && !detailOpen && !cleanup && <Notice>{task.error}</Notice>}
        {error && <Notice>{error}</Notice>}
        <DataTable
          headers={[
            "时间",
            "操作者 / 来源",
            "操作",
            "资源 / 对象",
            "结果",
            "IP / 耗时",
            "操作",
          ]}
          empty={!rows.length}
        >
          {rows.map((row) => (
            <Row key={row.id}>
              <Cell>{date(row.created_at)}</Cell>
              <Cell>
                <strong>{row.actor_name}</strong>
                <small className="block">
                  {actors[row.actor_kind] ?? row.actor_kind} ·{" "}
                  {sources[row.source] ?? row.source}
                </small>
              </Cell>
              <Cell>
                {row.summary}
                <small className="block audit-code">{row.action}</small>
              </Cell>
              <Cell>
                {resources[row.resource] ?? row.resource}
                <small className="block audit-target">
                  {row.target_id || "—"}
                </small>
              </Cell>
              <Cell>
                <span className={`audit-result ${row.result}`}>
                  {results[row.result] ?? row.result}
                </span>
                <small className="block">
                  {row.status_code ? `HTTP ${row.status_code}` : "系统事件"}
                </small>
              </Cell>
              <Cell>
                {row.ip || "—"}
                <small className="block">
                  {row.duration_ms == null ? "—" : `${row.duration_ms} ms`}
                </small>
              </Cell>
              <Cell>
                <RowAction
                  label="详情"
                  icon={Eye}
                  onClick={() => void open(row.id)}
                />
              </Cell>
            </Row>
          ))}
        </DataTable>
      </PagePanel>
      <Dialog open={detailOpen} onOpenChange={setDetailOpen}>
        <DialogContent className="audit-detail-dialog">
          <DialogTitle>审计日志详情</DialogTitle>
          <DialogDescription>
            操作身份、请求关联和安全摘要；不包含凭据及原始请求内容。
          </DialogDescription>
          {task.error && <Notice>{task.error}</Notice>}
          {details ? (
            <>
              <ReadonlyInfo
                entries={{
                  时间: date(details.created_at),
                  完成时间: date(details.finished_at),
                  操作者: details.actor_name,
                  操作者ID: details.actor_id,
                  身份: actors[details.actor_kind] ?? details.actor_kind,
                  操作: details.summary,
                  资源: resources[details.resource] ?? details.resource,
                  对象: details.target_id,
                  结果: results[details.result],
                  状态码: details.status_code,
                  耗时:
                    details.duration_ms == null
                      ? "—"
                      : `${details.duration_ms} ms`,
                  IP: details.ip,
                  请求ID: details.request_id,
                  请求方法: details.method,
                  路由: details.route,
                  "User Agent": details.user_agent,
                }}
              />
              <h3>变更摘要</h3>
              <pre className="audit-details-json">
                {JSON.stringify(details.details, null, 2)}
              </pre>
            </>
          ) : (
            <Skeleton className="h-32 w-full" />
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setDetailOpen(false)}>
              关闭
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog
        open={cleanup}
        onOpenChange={(value) => {
          if (!task.pending) setCleanup(value);
        }}
      >
        <DialogContent>
          <DialogTitle>清理旧审计日志</DialogTitle>
          <DialogDescription>
            永久删除保留天数之前的已完成记录，结果未确认的记录会保留。仅执行本次清理，不改变自动保留策略。
          </DialogDescription>
          <p>
            自动保留策略：
            {data?.retention_days ? `${data.retention_days} 天` : "保留全部"}
          </p>
          <TextField
            label="保留天数"
            type="number"
            min={1}
            max={3650}
            value={days}
            onValue={setDays}
          />
          {task.error && <Notice>{task.error}</Notice>}
          <DialogFooter>
            <Button
              variant="outline"
              disabled={task.pending}
              onClick={() => setCleanup(false)}
            >
              取消
            </Button>
            <Button
              variant="destructive"
              disabled={
                task.pending ||
                !Number.isInteger(Number(days)) ||
                Number(days) < 1 ||
                Number(days) > 3650
              }
              onClick={async () => {
                if (
                  await task.run(
                    () => api.cleanupAudit(Number(days)),
                    "旧日志已清理",
                  )
                )
                  setCleanup(false);
              }}
            >
              确认清理
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

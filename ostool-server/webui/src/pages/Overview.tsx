import {
  Activity,
  ArrowRight,
  CalendarClock,
  CircuitBoard,
  Cpu,
  Database,
  HardDrive,
  MemoryStick,
  Network,
  Server,
  Users,
  ShieldCheck,
  RefreshCw,
} from "lucide-react";
import { Link } from "react-router-dom";
import { admin, useConnection, useResource } from "@/api/events";
import { useEdition } from "@/edition";
import { PagePanel } from "@/components/page-panel";
import { ListToolbar } from "@/components/list-toolbar";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { DataTable, Row, Cell, Notice } from "@/components/forms";
import type { DashboardInfo } from "@/types/api";
const bytes = (value: number | null | undefined) => {
  if (value == null) return "不可用";
  if (value < 1024) return `${value.toFixed(0)} B`;
  const index = Math.min(3, Math.floor(Math.log(value) / Math.log(1024)));
  return `${(value / 1024 ** index).toFixed(1)} ${["B", "KiB", "MiB", "GiB"][index]}`;
};
const date = (value: string | null | undefined) =>
  value
    ? new Date(value).toLocaleString("zh-CN", { hour12: false })
    : "尚未执行";
const duration = (seconds: number) =>
  `${Math.floor(seconds / 86400)} 天 ${Math.floor((seconds % 86400) / 3600)} 小时 ${Math.floor((seconds % 3600) / 60)} 分`;
function Metric({
  label,
  value,
  detail,
  icon,
  href,
  tone = "blue",
}: {
  label: string;
  value: React.ReactNode;
  detail?: React.ReactNode;
  icon: React.ReactNode;
  href?: string;
  tone?: string;
}) {
  const content = (
    <>
      <div className="dashboard-metric-head">
        <span className={`dashboard-metric-icon ${tone}`}>{icon}</span>
        <span>{label}</span>
        {href && <ArrowRight size={14} />}
      </div>
      <strong>{value}</strong>
      <small>{detail}</small>
    </>
  );
  return href ? (
    <Link to={href} className="dashboard-metric">
      {content}
    </Link>
  ) : (
    <article className="dashboard-metric">{content}</article>
  );
}
function Gauge({ value }: { value: number | null | undefined }) {
  return (
    <div className="dashboard-gauge">
      <i style={{ width: `${Math.max(0, Math.min(100, value ?? 0))}%` }} />
    </div>
  );
}
function Facts({ items }: { items: [string, React.ReactNode][] }) {
  return (
    <dl className="dashboard-facts">
      {items.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}
function Trend({
  history,
  fields,
  labels,
  format,
  percent = false,
}: {
  history: DashboardInfo["runtime"]["history"];
  fields: ("cpu_percent" | "memory_percent" | "rx" | "tx")[];
  labels: string[];
  format: (value: number) => string;
  percent?: boolean;
}) {
  const values = history.flatMap((point) =>
      fields
        .map((field) => point[field])
        .filter((value): value is number => value != null),
    ),
    max = percent ? 100 : Math.max(1, ...values);
  return (
    <div className="dashboard-trend">
      <div className="trend-legend">
        {labels.map((label, index) => (
          <span key={label}>
            <i className={`series-${index}`} />
            {label}
          </span>
        ))}
        <small>{percent ? "0–100%" : `峰值 ${format(max)}`}</small>
      </div>
      {values.length < 2 ? (
        <div className="dashboard-empty-chart">正在采集真实运行数据…</div>
      ) : (
        <svg
          viewBox="0 0 600 140"
          role="img"
          aria-label={labels.join("与") + "趋势"}
        >
          {[0, 35, 70, 105, 140].map((y) => (
            <line key={y} x1="0" x2="600" y1={y} y2={y} />
          ))}
          {fields.map((field, series) => {
            let open = false;
            const path = history
              .map((point, index) => {
                const value = point[field];
                if (value == null) {
                  open = false;
                  return "";
                }
                const command = open ? "L" : "M";
                open = true;
                return `${command}${(index * 600) / Math.max(1, history.length - 1)},${140 - (value / max) * 130}`;
              })
              .join(" ");
            return (
              <g key={field}>
                <path className={`series-${series}`} d={path} />
                {history.map((point, index) =>
                  point[field] == null ? null : (
                    <circle
                      key={point.at}
                      className={`series-${series}`}
                      cx={(index * 600) / Math.max(1, history.length - 1)}
                      cy={140 - ((point[field] ?? 0) / max) * 130}
                      r="3"
                    >
                      <title>
                        {date(point.at)} · {labels[series]}{" "}
                        {format(point[field]!)}
                      </title>
                    </circle>
                  ),
                )}
              </g>
            );
          })}
        </svg>
      )}
      <div className="trend-range">
        <span>
          {history[0]
            ? new Date(history[0].at).toLocaleTimeString("zh-CN")
            : "—"}
        </span>
        <span>最近 {history.length} 次采样</span>
        <span>
          {history.at(-1)
            ? new Date(history.at(-1)!.at).toLocaleTimeString("zh-CN")
            : "—"}
        </span>
      </div>
    </div>
  );
}
export default function Overview({
  view = "runtime",
}: {
  view?: "runtime" | "jobs" | "operations";
}) {
  const data = useResource("dashboard"),
    connection = useConnection(),
    publicEdition = useEdition().edition === "public";
  return (
    <PagePanel
      toolbar={
        <ListToolbar
          actions={
            <>
              <Button asChild>
                <Link to="/boards/new">
                  <CircuitBoard size={16} />
                  新建开发板
                </Link>
              </Button>
              <Button asChild variant="outline">
                <Link to="/settings">系统设置</Link>
              </Button>
            </>
          }
        >
          <span className={`dashboard-connection ${connection}`} role="status">
            <i />
            {connection === "connected"
              ? "实时已连接"
              : connection === "connecting"
                ? "正在连接…"
                : "连接中断"}
          </span>
          {connection !== "connected" && (
            <Button variant="outline" onClick={admin.restart}>
              <RefreshCw size={15} />
              重连
            </Button>
          )}
        </ListToolbar>
      }
      footer={
        <div className="dashboard-footer">
          <span>
            {data
              ? `${data.operations.boards} 台开发板 · ${data.operations.board_states.idle ?? 0} 台可用 · ${data.operations.sessions} 个会话`
              : "正在同步…"}
          </span>
          <span>
            采样时间{" "}
            {data?.runtime.sample.sampled_at
              ? date(data.runtime.sample.sampled_at)
              : "正在采集"}
          </span>
        </div>
      }
    >
      {!data ? (
        <Skeleton className="h-64 w-full" />
      ) : (
        <div className="dashboard-content">
          {view === "runtime" ? (
            <Runtime data={data} />
          ) : view === "operations" ? (
            <Operations data={data} publicEdition={publicEdition} />
          ) : (
            <Jobs data={data} />
          )}
        </div>
      )}
    </PagePanel>
  );
}
function Runtime({ data }: { data: DashboardInfo }) {
  const { runtime, audit } = data,
    sample = runtime.sample;
  return (
    <>
      <div className="dashboard-banner">
        <div>
          <span className="dashboard-banner-icon">
            <Server />
          </span>
          <div>
            <strong>{sample.hostname || "ostool-server"}</strong>
            <p>
              {runtime.platform} / {runtime.arch} · v{runtime.version} · PID{" "}
              {runtime.pid}
            </p>
          </div>
        </div>
        <span
          className={`dashboard-health ${sample.database_ok ? "healthy" : "warning"}`}
        >
          {sample.database_ok ? "服务运行正常" : "数据库状态待确认"}
        </span>
      </div>
      <div className="dashboard-metrics">
        <Metric
          label="持续运行"
          value={duration(runtime.uptime_secs)}
          detail={`启动于 ${date(runtime.started_at)}`}
          icon={<Activity />}
        />
        <Metric
          label="宿主 CPU"
          value={
            sample.cpu_percent == null
              ? "采集中"
              : `${sample.cpu_percent.toFixed(1)}%`
          }
          detail={
            <>
              <Gauge value={sample.cpu_percent} />
              {runtime.logical_cpus ?? "—"} 个可用逻辑 CPU
            </>
          }
          icon={<Cpu />}
        />
        <Metric
          label="宿主内存"
          value={
            sample.memory_total == null
              ? "不可用"
              : bytes(
                  sample.memory_total -
                    (sample.memory_available ?? sample.memory_total),
                )
          }
          detail={
            <>
              <Gauge
                value={
                  sample.memory_total
                    ? (100 *
                        (sample.memory_total -
                          (sample.memory_available ?? sample.memory_total))) /
                      sample.memory_total
                    : null
                }
              />
              总计 {bytes(sample.memory_total)}
            </>
          }
          icon={<MemoryStick />}
          tone="violet"
        />
        <Metric
          label="数据目录可用空间"
          value={bytes(sample.disk_available)}
          detail={`磁盘总计 ${bytes(sample.disk_total)}`}
          icon={<HardDrive />}
          tone="green"
        />
      </div>
      <div className="dashboard-columns">
        <section className="dashboard-card">
          <header>
            <h2>资源使用趋势</h2>
            <span>宿主 CPU / 内存</span>
          </header>
          <Trend
            history={runtime.history}
            fields={["cpu_percent", "memory_percent"]}
            labels={["CPU", "内存"]}
            percent
            format={(value) => `${value.toFixed(1)}%`}
          />
        </section>
        <section className="dashboard-card">
          <header>
            <h2>网络流量</h2>
            <span>{sample.interface || "未选择网卡"}</span>
          </header>
          <Trend
            history={runtime.history}
            fields={["rx", "tx"]}
            labels={["接收", "发送"]}
            format={(value) => `${bytes(value)}/s`}
          />
        </section>
      </div>
      <div className="dashboard-columns">
        <section className="dashboard-card">
          <header>
            <h2>服务运行状态</h2>
            <Database size={16} />
          </header>
          <Facts
            items={[
              [
                "PostgreSQL",
                <span
                  className={sample.database_ok ? "status-good" : "status-bad"}
                >
                  {sample.database_ok ? "连接正常" : "未连接 / 采样中"}
                </span>,
              ],
              [
                "数据库响应",
                sample.database_ok ? `${sample.database_latency_ms} ms` : "—",
              ],
              [
                "连接池",
                `${sample.pool_size ?? "—"} 个连接 · ${sample.pool_idle ?? "—"} 个空闲`,
              ],
              ["进程内存 RSS", bytes(sample.process_rss)],
              ["SSE 连接", runtime.http.sse_connections],
              ["平均请求耗时", `${runtime.http.mean_latency_ms.toFixed(1)} ms`],
            ]}
          />
        </section>
        <section className="dashboard-card">
          <header>
            <h2>请求与审计</h2>
            <ShieldCheck size={16} />
          </header>
          <Facts
            items={[
              ["HTTP 请求", runtime.http.requests],
              ["请求错误", runtime.http.errors],
              ["正在处理", runtime.http.inflight],
              [
                "审计写入",
                <span className={audit.healthy ? "status-good" : "status-bad"}>
                  {audit.healthy ? "正常" : "待确认 / 写入异常"}
                </span>,
              ],
              ["累计写入失败", audit.write_failures],
              [
                "保留策略",
                data.audit_retention_days
                  ? `${data.audit_retention_days} 天`
                  : "保留全部",
              ],
            ]}
          />
          <Link className="dashboard-card-link" to="/audits">
            查看审计日志 <ArrowRight size={14} />
          </Link>
        </section>
      </div>
    </>
  );
}
function Operations({
  data,
  publicEdition,
}: {
  data: DashboardInfo;
  publicEdition: boolean;
}) {
  const operations = data.operations;
  return (
    <>
      <div className="dashboard-metrics">
        <Metric
          label="开发板总数"
          value={operations.boards}
          detail={`${operations.board_states.disabled ?? 0} 台已禁用`}
          icon={<CircuitBoard />}
          href="/boards"
        />
        <Metric
          label="可用开发板"
          value={operations.board_states.idle ?? 0}
          detail={`${operations.board_states.using ?? 0} 台使用中`}
          icon={<Activity />}
          href="/boards"
          tone="green"
        />
        <Metric
          label="活跃会话"
          value={operations.sessions}
          detail="实时硬件占用"
          icon={<Network />}
          href="/sessions"
          tone="violet"
        />
        <Metric
          label="在线发现设备"
          value={operations.online_devices}
          detail={`${operations.conflicting_devices} 台冲突 · ${operations.devices} 台已发现`}
          icon={<Server />}
          href="/boards"
        />
      </div>
      {publicEdition && (
        <div className="dashboard-metrics">
          <Metric
            label="注册用户"
            value={operations.users ?? "—"}
            detail={`${operations.enabled_users ?? "—"} 位已启用`}
            icon={<Users />}
            href="/users"
          />
          <Metric
            label="使用期内租赁"
            value={operations.active_rentals ?? "—"}
            detail={`${operations.scheduled_rentals ?? "—"} 条即将开始`}
            icon={<CalendarClock />}
            href="/rentals"
            tone="green"
          />
          <Metric
            label="待审核申请"
            value={operations.pending_requests ?? "—"}
            detail="需要管理员审核"
            icon={<CalendarClock />}
            href="/rental-requests"
            tone="amber"
          />
          <Metric
            label="已取消租赁"
            value={operations.cancelled_rentals ?? "—"}
            detail="历史记录保留"
            icon={<CalendarClock />}
            href="/rentals"
            tone="violet"
          />
        </div>
      )}
      {operations.database_available === false && (
        <Notice>运营统计暂时无法连接数据库，实时板卡状态仍会保留。</Notice>
      )}
      <div className="dashboard-columns">
        <section className="dashboard-card">
          <header>
            <h2>开发板状态分布</h2>
            <Link to="/boards">查看开发板 →</Link>
          </header>
          <div className="dashboard-breakdown">
            {[
              ["idle", "空闲"],
              ["using", "使用中"],
              ["releasing", "释放中"],
              ["error", "释放失败"],
              ["disabled", "已禁用"],
            ].map(([key, label]) => (
              <div key={key}>
                <span>{label}</span>
                <i>
                  <b
                    className={`board-state-${key}`}
                    style={{
                      width: `${(100 * (operations.board_states[key] ?? 0)) / Math.max(1, operations.boards)}%`,
                    }}
                  />
                </i>
                <strong>{operations.board_states[key] ?? 0}</strong>
              </div>
            ))}
          </div>
        </section>
        <section className="dashboard-card">
          <header>
            <h2>近期运营活动</h2>
            <span>最近 24 小时</span>
          </header>
          <Facts
            items={[
              ["审计记录", operations.recent_audits ?? "—"],
              ["失败 / 拒绝", operations.recent_failures ?? "—"],
              ["DTB 文件", operations.dtbs ?? "—"],
              [
                "活跃 OTA 任务",
                data.ota_jobs.filter(
                  (job) =>
                    ![
                      "succeeded",
                      "rolled_back",
                      "failed",
                      "cancelled",
                    ].includes(job.phase),
                ).length,
              ],
            ]}
          />
          <Link className="dashboard-card-link" to="/audits">
            查看操作记录 <ArrowRight size={14} />
          </Link>
        </section>
      </div>
    </>
  );
}
function Jobs({ data }: { data: DashboardInfo }) {
  const { runtime, power_actions: power, ota_jobs: ota } = data;
  return (
    <>
      <div className="dashboard-metrics">
        <Metric
          label="定时维护任务"
          value={runtime.tasks.length}
          detail="服务器实际注册任务"
          icon={<Activity />}
        />
        <Metric
          label="进行中的电源操作"
          value={power.filter((task) => task.state === "running").length}
          detail="异步电源执行"
          icon={<Cpu />}
        />
        <Metric
          label="活跃 OTA 任务"
          value={
            ota.filter(
              (job) =>
                !["succeeded", "rolled_back", "failed", "cancelled"].includes(
                  job.phase,
                ),
            ).length
          }
          detail="板卡固件升级"
          icon={<Server />}
        />
        <Metric
          label="任务异常"
          value={
            runtime.tasks.filter((task) => task.last_ok === false).length +
            power.filter((task) => task.state === "failed").length +
            ota.filter((job) => job.phase === "failed").length
          }
          detail="请结合操作记录排查"
          icon={<ShieldCheck />}
          tone="amber"
        />
      </div>
      <section className="dashboard-card dashboard-task-list">
        <header>
          <h2>维护任务</h2>
          <span>运行计划与最近执行结果</span>
        </header>
        <DataTable
          headers={[
            "任务",
            "状态",
            "执行次数",
            "最近运行",
            "下一次运行",
            "耗时",
          ]}
          empty={!runtime.tasks.length}
        >
          {runtime.tasks.map((task) => (
            <Row key={task.id}>
              <Cell>
                <strong>{task.name}</strong>
                <small className="block">每 {task.interval_secs} 秒</small>
              </Cell>
              <Cell>
                {task.state === "running"
                  ? "执行中"
                  : task.last_ok === false
                    ? "最近执行失败"
                    : "等待下次运行"}
              </Cell>
              <Cell>{task.runs}</Cell>
              <Cell>{date(task.last_run)}</Cell>
              <Cell>{date(task.next_run)}</Cell>
              <Cell>
                {task.last_duration_ms == null
                  ? "—"
                  : `${task.last_duration_ms} ms`}
              </Cell>
            </Row>
          ))}
        </DataTable>
      </section>
      <section className="dashboard-card dashboard-task-list">
        <header>
          <h2>电源操作</h2>
          <span>当前进程内的异步任务</span>
        </header>
        <DataTable
          headers={["操作 ID", "动作", "结果", "说明"]}
          empty={!power.length}
        >
          {power.map((task) => (
            <Row key={task.id}>
              <Cell>{task.id}</Cell>
              <Cell>{task.action === "on" ? "上电" : "下电"}</Cell>
              <Cell>
                {{ running: "执行中", succeeded: "已完成", failed: "失败" }[
                  task.state
                ] ?? task.state}
              </Cell>
              <Cell>{task.message ?? "—"}</Cell>
            </Row>
          ))}
        </DataTable>
      </section>
      <section className="dashboard-card dashboard-task-list">
        <header>
          <h2>OTA 升级</h2>
          <span>持久化任务</span>
        </header>
        <DataTable
          headers={["开发板", "阶段", "镜像版本", "投递次数"]}
          empty={!ota.length}
        >
          {ota.map((job) => (
            <Row key={job.update_id}>
              <Cell>
                <Link to={`/boards/${encodeURIComponent(job.board_id)}`}>
                  {job.board_id}
                </Link>
              </Cell>
              <Cell>{job.phase}</Cell>
              <Cell>{job.image.version || job.image.sha256.slice(0, 12)}</Cell>
              <Cell>{job.delivery_attempts}</Cell>
            </Row>
          ))}
        </DataTable>
      </section>
    </>
  );
}

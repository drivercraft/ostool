import { useState, type FormEvent } from "react";
import {
  Link,
  NavLink,
  Navigate,
  Route,
  Routes,
  useLocation,
} from "react-router-dom";
import {
  CalendarClock,
  ClipboardList,
  KeyRound,
  LayoutDashboard,
  ShieldCheck,
  UserRound,
} from "lucide-react";
import { api, clearAuthToken, type AuthUser } from "@/api/client";
import {
  useAccountConnection,
  useAccountErrors,
  useAccountResource,
} from "@/api/events";
import { useSiteSession } from "@/site-session";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  TextField,
  SelectField,
  TextareaField,
  Notice,
  DataTable,
  Row,
  Cell,
  ConfirmAction,
  useAction,
} from "@/components/forms";
import { useListPage } from "@/components/list-pagination";
import type { RentalRequest } from "@/types/api";

export const requestStatusLabels: Record<RentalRequest["status"], string> = {
  pending: "待审核",
  approved: "已通过",
  rejected: "已拒绝",
  cancelled: "已撤回",
};
const rentalLabels: Record<string, string> = {
  active: "使用期内",
  scheduled: "未开始",
  expired: "已结束",
  cancelled: "已取消",
};
const date = (value: string) =>
  new Date(value).toLocaleString("zh-CN", { hour12: false });
const localDate = (offset: number) => {
  const d = new Date(Date.now() + offset);
  return new Date(d.getTime() - d.getTimezoneOffset() * 60000)
    .toISOString()
    .slice(0, 16);
};
const navigation = [
  ["/account", "工作台总览", LayoutDashboard],
  ["/account/profile", "基本信息", UserRound],
  ["/account/security", "账号安全", ShieldCheck],
  ["/account/rentals", "我的租赁", CalendarClock],
  ["/account/requests", "租赁申请", ClipboardList],
  ["/account/tokens", "访问令牌", KeyRound],
] as const;

export function MemberDeviceCatalog() {
  const catalog = useAccountResource("catalog");
  const [search, setSearch] = useState("");
  const filtered = (catalog ?? []).filter((b) =>
    `${b.id} ${b.board_type} ${b.tags.join(" ")}`
      .toLowerCase()
      .includes(search.toLowerCase()),
  );
  const page = useListPage(filtered, search);
  return (
    <section className="member-card member-list">
      <div className="member-list-tools">
        <h2>设备目录</h2>
        <TextField
          label="搜索设备"
          placeholder="板型、名称或标签"
          value={search}
          onValue={setSearch}
        />
      </div>
      {!catalog ? (
        <Skeleton className="h-32 w-full" />
      ) : (
        <DataTable
          headers={["开发板", "板型", "标签", "操作"]}
          empty={!filtered.length}
        >
          {page.rows.map((board) => (
            <Row key={board.id}>
              <Cell>
                <strong>{board.id}</strong>
              </Cell>
              <Cell>{board.board_type}</Cell>
              <Cell>{board.tags.join(" · ") || "—"}</Cell>
              <Cell>
                <Button asChild size="sm">
                  <Link
                    to={`/account/requests?board=${encodeURIComponent(board.id)}`}
                  >
                    申请租赁
                  </Link>
                </Button>
              </Cell>
            </Row>
          ))}
        </DataTable>
      )}
      <div className="member-list-footer">{page.footer}</div>
    </section>
  );
}

export default function MemberWorkspace() {
  const { user, checking, error } = useSiteSession();
  const connection = useAccountConnection(),
    errors = useAccountErrors();
  const location = useLocation();
  if (checking)
    return (
      <main className="member-loading">
        <Skeleton className="h-48 w-full" />
      </main>
    );
  if (!user)
    return error ? (
      <main className="member-loading">
        <Notice>{error}</Notice>
        <Link to="/login">重新登录</Link>
      </main>
    ) : (
      <Navigate
        replace
        to={`/login?redirect=${encodeURIComponent(location.pathname)}`}
      />
    );
  return (
    <main className="member-workspace">
      <aside className="member-sidebar">
        <div className="member-identity">
          <span className="site-avatar">
            {(user.display_name || user.email).slice(0, 1)}
          </span>
          <div>
            <strong>{user.display_name || "我的工作台"}</strong>
            <small>{user.email}</small>
          </div>
        </div>
        <nav aria-label="个人中心导航">
          {navigation.map(([path, label, Icon]) => (
            <NavLink key={path} to={path} end={path === "/account"}>
              <Icon size={17} />
              {label}
            </NavLink>
          ))}
        </nav>
        <p className="member-sidebar-note">你的开发板使用计划与账号设置</p>
      </aside>
      <div className="member-main">
        {connection === "reconnecting" && (
          <Notice>连接暂时中断，输入已保留，恢复后自动同步。</Notice>
        )}
        {Object.entries(errors).map(([key, message]) => (
          <Notice key={key}>{message}</Notice>
        ))}
        <Routes>
          <Route index element={<Overview user={user} />} />
          <Route path="profile" element={<Profile user={user} />} />
          <Route path="security" element={<Security />} />
          <Route path="rentals" element={<MyRentals />} />
          <Route path="requests" element={<MyRequests />} />
          <Route path="tokens" element={<Tokens />} />
          <Route path="*" element={<Navigate replace to="/account" />} />
        </Routes>
      </div>
    </main>
  );
}
function Heading({
  title,
  text,
  action,
}: {
  title: string;
  text: string;
  action?: React.ReactNode;
}) {
  return (
    <div className="member-heading">
      <div>
        <span className="section-label">MY WORKSPACE</span>
        <h1>{title}</h1>
        <p>{text}</p>
      </div>
      {action}
    </div>
  );
}
function Overview({ user }: { user: AuthUser }) {
  const rentals = useAccountResource("my_rentals") ?? [],
    requests = useAccountResource("my_requests") ?? [];
  return (
    <>
      <Heading
        title={`你好，${user.display_name || "开发者"}`}
        text="在这里安排开发板使用时间，管理你的账号。"
        action={
          <Button asChild>
            <Link to="/account/requests">申请租赁</Link>
          </Button>
        }
      />
      <div className="member-metrics">
        {[
          [
            "使用中的租赁",
            rentals.filter((r) => r.status === "active").length,
            "/account/rentals",
          ],
          [
            "即将开始",
            rentals.filter((r) => r.status === "scheduled").length,
            "/account/rentals",
          ],
          [
            "待审核申请",
            requests.filter((r) => r.status === "pending").length,
            "/account/requests",
          ],
        ].map(([label, count, path]) => (
          <Link key={label} to={String(path)}>
            <span>{label}</span>
            <strong>{count}</strong>
            <small>查看详情 →</small>
          </Link>
        ))}
      </div>
      <section className="member-card">
        <h2>当前账号</h2>
        <dl className="member-facts">
          <div>
            <dt>显示名称</dt>
            <dd>{user.display_name || "未设置"}</dd>
          </div>
          <div>
            <dt>邮箱</dt>
            <dd>{user.email}</dd>
          </div>
          <div>
            <dt>账号类型</dt>
            <dd>{user.is_admin ? "管理员" : "成员"}</dd>
          </div>
        </dl>
        <Button asChild variant="outline">
          <Link to="/account/profile">编辑个人资料</Link>
        </Button>
      </section>
      <section className="member-card member-cli">
        <div>
          <h2>从 CLI 连接开发板</h2>
          <p>租赁审核通过后，在对应使用时段内登录 ostool 并创建板卡会话。</p>
        </div>
        <code>ostool login</code>
        <Link to="/account/tokens">管理个人访问令牌 →</Link>
      </section>
    </>
  );
}
function Profile({ user }: { user: AuthUser }) {
  const [email, setEmail] = useState(user.email),
    [name, setName] = useState(user.display_name ?? ""),
    [password, setPassword] = useState("");
  const task = useAction();
  async function save(e: FormEvent) {
    e.preventDefault();
    if (
      await task.run(
        () =>
          api.updateProfile({
            email,
            display_name: name,
            current_password: password || undefined,
          }),
        "个人资料已保存",
      )
    )
      setPassword("");
  }
  return (
    <>
      <Heading title="基本信息" text="管理对外显示的名称和登录邮箱。" />
      <form className="member-card member-form" onSubmit={save}>
        {task.error && <Notice>{task.error}</Notice>}
        <TextField
          label="显示名称"
          value={name}
          onValue={setName}
          maxLength={120}
        />
        <TextField
          label="邮箱"
          type="email"
          required
          value={email}
          onValue={setEmail}
          autoComplete="email"
        />
        {email !== user.email && (
          <TextField
            label="当前密码（修改邮箱时必填）"
            type="password"
            required
            maxLength={1024}
            value={password}
            onValue={setPassword}
            autoComplete="current-password"
          />
        )}
        <div className="member-form-actions">
          <Button disabled={task.pending}>保存资料</Button>
          <Button
            type="button"
            variant="outline"
            onClick={() => {
              setEmail(user.email);
              setName(user.display_name ?? "");
              setPassword("");
            }}
          >
            取消修改
          </Button>
        </div>
      </form>
    </>
  );
}
function Security() {
  const [current, setCurrent] = useState(""),
    [password, setPassword] = useState(""),
    [confirmation, setConfirmation] = useState("");
  const task = useAction();
  async function save(e: FormEvent) {
    e.preventDefault();
    await task.run(async () => {
      if (password !== confirmation) throw new Error("两次输入的密码不一致");
      await api.changePassword(current, password);
      clearAuthToken();
      window.location.assign("/login?password_changed=1");
    });
  }
  return (
    <>
      <Heading
        title="账号安全"
        text="修改登录密码，保护你的账号和开发板访问权限。"
      />
      <form className="member-card member-form" onSubmit={save}>
        {task.error && <Notice>{task.error}</Notice>}
        <TextField
          label="当前密码"
          type="password"
          required
          maxLength={1024}
          value={current}
          onValue={setCurrent}
          autoComplete="current-password"
        />
        <TextField
          label="新密码"
          type="password"
          required
          minLength={12}
          maxLength={1024}
          value={password}
          onValue={setPassword}
          autoComplete="new-password"
        />
        <TextField
          label="确认新密码"
          type="password"
          required
          minLength={12}
          maxLength={1024}
          value={confirmation}
          onValue={setConfirmation}
          autoComplete="new-password"
        />
        <p className="muted">
          密码至少 12 个字符。修改后所有登录与个人访问令牌失效，需要重新登录。
        </p>
        <div className="member-form-actions">
          <Button disabled={task.pending}>更新密码</Button>
        </div>
      </form>
    </>
  );
}
function MyRentals() {
  const rentals = useAccountResource("my_rentals");
  const [search, setSearch] = useState(""),
    [status, setStatus] = useState("");
  const filtered = (rentals ?? []).filter(
    (r) =>
      (!status || r.status === status) &&
      `${r.board_id} ${r.notes}`.toLowerCase().includes(search.toLowerCase()),
  );
  const page = useListPage(filtered, JSON.stringify([search, status]));
  return (
    <>
      <Heading
        title="我的租赁"
        text="查看分配给你的开发板及使用时段。"
        action={
          <Button asChild>
            <Link to="/account/requests">申请租赁</Link>
          </Button>
        }
      />
      <section className="member-card member-list">
        <div className="member-list-tools">
          <TextField
            label="搜索我的租赁"
            placeholder="开发板或备注"
            value={search}
            onValue={setSearch}
          />
          <SelectField
            label="租赁状态"
            value={status}
            onValue={setStatus}
            options={[
              { value: "", label: "全部状态" },
              ...Object.entries(rentalLabels).map(([value, label]) => ({
                value,
                label,
              })),
            ]}
          />
        </div>
        {!rentals ? (
          <Skeleton className="h-40 w-full" />
        ) : (
          <DataTable
            headers={["开发板", "使用时间", "状态", "备注"]}
            empty={!filtered.length}
          >
            {page.rows.map((r) => (
              <Row key={r.id}>
                <Cell>
                  <strong>{r.board_id}</strong>
                </Cell>
                <Cell>
                  {date(r.starts_at)}
                  <br />
                  {date(r.ends_at)}
                </Cell>
                <Cell>
                  <span className={`member-status ${r.status}`}>
                    {rentalLabels[r.status]}
                  </span>
                </Cell>
                <Cell>{r.notes || "—"}</Cell>
              </Row>
            ))}
          </DataTable>
        )}
        <div className="member-list-footer">{page.footer}</div>
      </section>
    </>
  );
}
function MyRequests() {
  const location = useLocation();
  const catalog = useAccountResource("catalog") ?? [],
    requests = useAccountResource("my_requests");
  const [board, setBoard] = useState(
      () => new URLSearchParams(location.search).get("board") ?? "",
    ),
    [start, setStart] = useState(() => localDate(3600000)),
    [end, setEnd] = useState(() => localDate(7200000)),
    [notes, setNotes] = useState("");
  const [status, setStatus] = useState("");
  const task = useAction();
  const filtered = (requests ?? []).filter(
      (r) => !status || r.status === status,
    ),
    page = useListPage(filtered, status);
  async function submit(e: FormEvent) {
    e.preventDefault();
    if (
      await task.run(
        () =>
          api.applyRental({
            board_id: board || catalog[0]?.id || "",
            starts_at: new Date(start).toISOString(),
            ends_at: new Date(end).toISOString(),
            notes,
          }),
        "申请已提交，等待管理员审核",
      )
    )
      setNotes("");
  }
  return (
    <>
      <Heading
        title="租赁申请"
        text="选择开发板和使用时段，提交后由管理员审核。"
      />
      <form className="member-card member-form" onSubmit={submit}>
        <h2>申请开发板</h2>
        {task.error && <Notice>{task.error}</Notice>}
        <SelectField
          label="申请开发板"
          value={board || catalog[0]?.id || ""}
          onValue={setBoard}
          options={catalog.map((b) => ({
            value: b.id,
            label: `${b.id} · ${b.board_type}`,
          }))}
        />
        <div className="member-form-columns">
          <TextField
            label="开始时间"
            type="datetime-local"
            required
            value={start}
            onValue={setStart}
          />
          <TextField
            label="结束时间"
            type="datetime-local"
            required
            value={end}
            onValue={setEnd}
          />
        </div>
        <TextareaField label="申请说明" value={notes} onValue={setNotes} />
        <p className="muted">
          提交申请不会预占或启动开发板；审核通过后可在租赁时段内使用。
        </p>
        <div className="member-form-actions">
          <Button disabled={task.pending || !catalog.length}>提交申请</Button>
        </div>
      </form>
      <section className="member-card member-list">
        <div className="member-list-tools">
          <h2>我的申请</h2>
          <SelectField
            label="申请状态"
            value={status}
            onValue={setStatus}
            options={[
              { value: "", label: "全部状态" },
              ...Object.entries(requestStatusLabels).map(([value, label]) => ({
                value,
                label,
              })),
            ]}
          />
        </div>
        {!requests ? (
          <Skeleton className="h-40 w-full" />
        ) : (
          <DataTable
            headers={["开发板 / 说明", "申请时段", "状态 / 审核意见", "操作"]}
            empty={!filtered.length}
          >
            {page.rows.map((r) => (
              <Row key={r.id}>
                <Cell>
                  <strong>{r.board_id}</strong>
                  <small className="block">{r.notes || "—"}</small>
                </Cell>
                <Cell>
                  {date(r.starts_at)}
                  <br />
                  {date(r.ends_at)}
                </Cell>
                <Cell>
                  <span className={`member-status ${r.status}`}>
                    {requestStatusLabels[r.status]}
                  </span>
                  <small className="block">{r.decision_note || "—"}</small>
                </Cell>
                <Cell>
                  {r.status === "pending" && (
                    <ConfirmAction
                      label="撤回申请"
                      description="撤回后管理员将无法批准这条申请。"
                      triggerVariant="ghost"
                      action={() => api.withdrawRentalRequest(r.id)}
                    />
                  )}
                </Cell>
              </Row>
            ))}
          </DataTable>
        )}
        <div className="member-list-footer">{page.footer}</div>
      </section>
    </>
  );
}
function Tokens() {
  const tokens = useAccountResource("personal_tokens");
  const [name, setName] = useState("ostool CLI"),
    [days, setDays] = useState("90"),
    [secret, setSecret] = useState("");
  const task = useAction();
  async function create(e: FormEvent) {
    e.preventDefault();
    setSecret("");
    await task.run(async () => {
      const result = await api.createPersonalAccessToken(name, Number(days));
      setSecret(result.token);
    }, "个人访问令牌已创建");
  }
  return (
    <>
      <Heading
        title="个人访问令牌"
        text="为 ostool CLI 创建、查看和撤销访问凭据。"
      />
      <form className="member-card member-form" onSubmit={create}>
        {task.error && <Notice>{task.error}</Notice>}
        <TextField
          label="令牌名称"
          value={name}
          required
          maxLength={120}
          onValue={setName}
        />
        <TextField
          label="有效期（天）"
          type="number"
          required
          min={1}
          max={3650}
          value={days}
          onValue={setDays}
        />
        <Button disabled={task.pending}>创建令牌</Button>
        {secret && (
          <div className="token-result" role="status">
            <strong>令牌仅显示一次，请立即复制：</strong>
            <code>{secret}</code>
            <span>用于 ostool login --with-token</span>
          </div>
        )}
      </form>
      <section className="member-card member-list">
        <h2 className="member-list-title">已有令牌</h2>
        {!tokens ? (
          <Skeleton className="h-24 w-full" />
        ) : (
          <DataTable
            headers={["名称", "创建时间", "过期时间", "操作"]}
            empty={!tokens.length}
          >
            {tokens.map((t) => (
              <Row key={t.id}>
                <Cell>{t.name}</Cell>
                <Cell>{date(t.created_at)}</Cell>
                <Cell>{t.expires_at ? date(t.expires_at) : "不过期"}</Cell>
                <Cell>
                  <ConfirmAction
                    label="撤销令牌"
                    description="撤销后，使用该令牌的 CLI 将无法继续认证。"
                    action={() => api.revokePersonalAccessToken(t.id)}
                  />
                </Cell>
              </Row>
            ))}
          </DataTable>
        )}
      </section>
    </>
  );
}

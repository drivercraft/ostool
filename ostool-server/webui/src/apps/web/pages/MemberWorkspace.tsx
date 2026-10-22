import { SecuritySettings } from "@/components/security-settings";
import { useId, useState, type FormEvent } from "react";
import { requestStatusLabels } from "@/types/api";
import {
  Link,
  NavLink,
  Navigate,
  Route,
  Routes,
  useLocation,
  useNavigate,
} from "react-router-dom";
import {
  CalendarClock,
  ClipboardList,
  KeyRound,
  LayoutDashboard,
  ShieldCheck,
  UserRound,
  Undo2,
} from "lucide-react";
import { api, clearBrowserSession, type AuthUser } from "@/api/client";
import {
  useAccountConnection,
  useAccountErrors,
  useAccountResource,
} from "@/api/events";
import { useSiteSession } from "@/apps/web/session";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  TextField,
  SelectField,
  Notice,
  DataTable,
  Row,
  Cell,
  ConfirmAction,
  useAction,
} from "@/components/forms";
import { useListPage } from "@/components/list-pagination";
import { ListLayoutProvider, ListLayoutToggle } from "@/components/list-layout";
import { PagePanel } from "@/components/page-panel";
import type { CatalogBoard } from "@/types/api";
import { RentalCalendar } from "@/components/rental-calendar";
import {
  localDateTime,
  rentalConflicts,
  type RentalWindow,
} from "@/utils/rental-calendar";

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

export default function MemberWorkspace() {
  const { user, checking, error } = useSiteSession();
  const connection = useAccountConnection(),
    errors = useAccountErrors();
  const location = useLocation();
  if (checking)
    return (
      <main className="member-workspace" aria-busy="true">
        <MemberSidebar />
        <div className="member-main">
          <Skeleton className="h-20 w-full" />
          <div className="startup-metrics">
            {[0, 1, 2].map((key) => (
              <Skeleton key={key} className="h-28 w-full" />
            ))}
          </div>
          <Skeleton className="h-72 w-full" />
        </div>
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
        to={`/login?redirect=${encodeURIComponent(location.pathname + location.search)}`}
      />
    );
  return (
    <main className="member-workspace">
      <MemberSidebar user={user} />
      <div
        className={
          location.pathname.endsWith("/requests/new")
            ? "member-main member-editor-main"
            : "member-main"
        }
      >
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
          <Route
            path="requests/new"
            element={<RentalRequestEditor user={user} />}
          />
          <Route path="tokens" element={<Tokens />} />
          <Route path="*" element={<Navigate replace to="/account" />} />
        </Routes>
      </div>
    </main>
  );
}
function MemberSidebar({ user }: { user?: AuthUser }) {
  return (
    <aside className="member-sidebar">
      <div className="member-identity">
        {user ? (
          <span className="site-avatar">
            {(user.display_name || user.username).slice(0, 1).toUpperCase()}
          </span>
        ) : (
          <Skeleton className="size-9" />
        )}
        <div>
          <strong>
            {user?.display_name || user?.username || "个人工作台"}
          </strong>
          <small>{user ? user.email : "正在验证登录状态…"}</small>
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
            <Link to="/account/requests/new">申请租赁</Link>
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
        <TextField label="用户名" value={user.username} readOnly hint="" />
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
      clearBrowserSession();
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
          minLength={15}
          maxLength={1024}
          value={password}
          onValue={setPassword}
          autoComplete="new-password"
        />
        <TextField
          label="确认新密码"
          type="password"
          required
          minLength={15}
          maxLength={1024}
          value={confirmation}
          onValue={setConfirmation}
          autoComplete="new-password"
        />
        <p className="muted">
          密码至少 15 个字符。修改后所有登录与个人访问令牌失效，需要重新登录。
        </p>
        <div className="member-form-actions">
          <Button disabled={task.pending}>更新密码</Button>
        </div>
      </form>
      <SecuritySettings />
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
            <Link to="/account/requests/new">申请租赁</Link>
          </Button>
        }
      />
      <MemberList storageKey="my-rentals">
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
          <ListLayoutToggle />
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
      </MemberList>
    </>
  );
}
function RentalRequestEditor({ user }: { user: AuthUser }) {
  const catalog = useAccountResource("catalog"),
    availability = useAccountResource("availability");
  if (!catalog || !availability)
    return (
      <PagePanel>
        <Skeleton className="h-48 w-full" />
      </PagePanel>
    );
  if (!catalog.length)
    return (
      <PagePanel
        className="rental-request-panel"
        footer={
          <Button asChild variant="outline">
            <Link to="/account/requests">取消</Link>
          </Button>
        }
      >
        <Notice>暂无可申请的设备，请联系管理员。</Notice>
      </PagePanel>
    );
  return (
    <RentalRequestForm
      user={user}
      catalog={catalog}
      availability={availability}
    />
  );
}
function RentalRequestForm({
  user,
  catalog,
  availability,
}: {
  user: AuthUser;
  catalog: CatalogBoard[];
  availability: RentalWindow[];
}) {
  const location = useLocation(),
    navigate = useNavigate(),
    formId = useId(),
    task = useAction();
  const [board, setBoard] = useState(() => {
    const requested = new URLSearchParams(location.search).get("board");
    return requested ?? catalog[0]?.id ?? "";
  });
  const [start, setStart] = useState(() => localDate(3600000)),
    [end, setEnd] = useState(() => localDate(7200000)),
    [notes, setNotes] = useState("");
  const bookings = availability.filter((item) => item.board_id === board);
  const conflicts = rentalConflicts(
    bookings,
    Date.parse(start),
    Date.parse(end),
  );
  const valid =
    Number.isFinite(Date.parse(start)) &&
    Number.isFinite(Date.parse(end)) &&
    Date.parse(start) < Date.parse(end);
  const boardExists = catalog.some((item) => item.id === board);
  return (
    <PagePanel
      className="rental-editor-panel rental-request-panel"
      toolbar={
        <form
          id={formId}
          className="rental-editor-form"
          onSubmit={(event) => {
            event.preventDefault();
            void task
              .run(async () => {
                if (!boardExists) throw new Error("请选择可申请的设备");
                if (!valid) throw new Error("开始时间必须早于结束时间");
                if (conflicts.length)
                  throw new Error("所选时段已有租赁，请选择空闲时段");
                await api.applyRental({
                  board_id: board,
                  starts_at: new Date(start).toISOString(),
                  ends_at: new Date(end).toISOString(),
                  notes,
                });
              }, "申请已提交，等待管理员审核")
              .then((ok) => {
                if (ok) navigate("/account/requests");
              });
          }}
        >
          <TextField
            label="申请用户"
            value={
              user.display_name
                ? user.display_name + " · " + user.email
                : user.email
            }
            readOnly
          />
          <SelectField
            label="申请开发板"
            value={board}
            onValue={setBoard}
            disabled={task.pending}
            options={catalog.map((item) => ({
              value: item.id,
              label: item.id + " · " + item.board_type,
            }))}
          />
          <TextField
            label="开始时间"
            type="datetime-local"
            step="0.001"
            required
            value={start}
            onValue={setStart}
          />
          <TextField
            label="结束时间"
            type="datetime-local"
            step="0.001"
            required
            value={end}
            onValue={setEnd}
          />
          <TextField
            label="申请说明"
            maxLength={2000}
            value={notes}
            onValue={setNotes}
          />
        </form>
      }
      footer={
        <div className="editor-footer">
          <span className="panel-status">
            {conflicts.length
              ? "所选时段已有租赁"
              : "申请不预占设备，通过审核后生效"}
          </span>
          <div className="row-actions">
            <Button asChild variant="outline">
              <Link to="/account/requests">取消</Link>
            </Button>
            <Button
              type="submit"
              form={formId}
              disabled={
                task.pending || !boardExists || !valid || conflicts.length > 0
              }
            >
              {task.pending ? "提交中…" : "提交申请"}
            </Button>
          </div>
        </div>
      }
    >
      {task.error && <Notice>{task.error}</Notice>}
      {!boardExists && <Notice>所选设备目前不可申请，请重新选择。</Notice>}
      {conflicts.length > 0 && (
        <Notice>所选时段已有租赁，请调整时间或在日历中选择空闲时段。</Notice>
      )}
      {boardExists && (
        <RentalCalendar
          bookings={bookings}
          startsAt={start}
          endsAt={end}
          onSelect={(from, to) => {
            setStart(localDateTime(from));
            setEnd(localDateTime(to));
          }}
        />
      )}
    </PagePanel>
  );
}
function MyRequests() {
  const requests = useAccountResource("my_requests");
  const [status, setStatus] = useState("");
  const filtered = (requests ?? []).filter(
    (item) => !status || item.status === status,
  );
  const page = useListPage(filtered, status);
  return (
    <>
      <Heading title="租赁申请" text="查看你的设备租赁申请及审核结果。" />
      <MemberList storageKey="my-requests">
        <div className="member-list-tools">
          <Button asChild className="member-create-request">
            <Link to="/account/requests/new">新增申请</Link>
          </Button>
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
          <ListLayoutToggle />
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
                      icon={Undo2}
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
      </MemberList>
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
          max={365}
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
      <MemberList storageKey="tokens">
        <div className="member-list-tools">
          <h2>已有令牌</h2>
          <ListLayoutToggle />
        </div>
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
      </MemberList>
    </>
  );
}

function MemberList({
  children,
  storageKey,
}: {
  children: React.ReactNode;
  storageKey: string;
}) {
  return (
    <ListLayoutProvider storageKey={`member:${storageKey}`}>
      <section className="member-card member-list">{children}</section>
    </ListLayoutProvider>
  );
}

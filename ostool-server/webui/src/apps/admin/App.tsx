import { accessToken, loginUrl } from "@/auth-session";
import AdminLogin from "@/apps/admin/pages/AdminLogin";
import { lazy, Suspense, useEffect, useState, type ReactNode } from "react";
import {
  BrowserRouter,
  NavLink,
  Navigate,
  Route,
  Routes,
  useLocation,
} from "react-router-dom";
import {
  LayoutDashboardIcon,
  CircuitBoardIcon,
  CalendarClockIcon,
  SettingsIcon,
  UserCircleIcon,
  ChevronRightIcon,
  ScrollTextIcon,
} from "lucide-react";
import { EditionProvider, useEdition } from "@/edition";
import { api, type AuthUser } from "@/api/client";
import { ThemeProvider } from "next-themes";
import { AdminTopbar } from "@/apps/admin/components/admin-topbar";
import { admin, useConnection, useErrors } from "@/api/events";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Toaster } from "@/components/ui/sonner";
import { Notice } from "@/components/forms";
const Boards = lazy(() => import("@/apps/admin/pages/Boards"));
const BoardEditor = lazy(() => import("@/apps/admin/pages/BoardEditor"));
const Overview = lazy(() => import("@/apps/admin/pages/Overview"));
const Audits = lazy(() => import("@/apps/admin/pages/Audits"));
const Dtbs = lazy(() => import("@/apps/admin/pages/Dtbs"));
const Sessions = lazy(() => import("@/apps/admin/pages/Sessions"));
const Settings = lazy(() => import("@/apps/admin/pages/SystemSettings"));
const Rentals = lazy(() => import("@/apps/admin/pages/Rentals"));
const RentalEditor = lazy(() => import("@/apps/admin/pages/RentalEditor"));
const Users = lazy(() => import("@/apps/admin/pages/Users"));
const Roles = lazy(() => import("@/apps/admin/pages/Roles"));
const RentalRequests = lazy(() => import("@/apps/admin/pages/RentalRequests"));
const boardNavigation = [
  ["boards", "开发板列表"],
  ["dtbs", "DTB 管理"],
  ["sessions", "会话租约"],
] as const;
const userNavigation = [
  ["users", "用户列表"],
  ["users/roles", "用户角色"],
] as const;
export default function App() {
  return (
    <ThemeProvider
      attribute="class"
      defaultTheme="system"
      enableSystem
      storageKey="ostool.admin.theme"
      disableTransitionOnChange
    >
      <EditionProvider>
        <AdminEntry />
      </EditionProvider>
    </ThemeProvider>
  );
}
function AdminEntry() {
  const config = useEdition();
  const loginPage = /^\/admin\/login\/?$/.test(window.location.pathname);
  const [gate, setGate] = useState<"checking" | "allowed" | "denied" | "error">(
    "checking",
  );
  const [message, setMessage] = useState("");
  const [currentUser, setCurrentUser] = useState<AuthUser>();
  useEffect(() => {
    let active = true;
    (async () => {
      if (config.edition === "local") {
        if (loginPage) {
          window.location.replace("/admin/overview");
          return;
        }
        if (active) setGate("allowed");
        return;
      }
      if (loginPage) return;
      if (!accessToken("admin")) {
        window.location.replace(loginUrl("admin"));
        return;
      }
      const user = await api.me();
      if (active) {
        setCurrentUser(user);
        setGate(user.is_admin ? "allowed" : "denied");
      }
    })().catch((e) => {
      if (active) {
        setGate("error");
        setMessage(e instanceof Error ? e.message : "无法验证登录状态");
      }
    });
    return () => {
      active = false;
    };
  }, [config, loginPage]);
  if (loginPage && config.edition === "public") return <AdminLogin />;
  if (gate === "checking")
    return (
      <div className="admin-gate">
        <Skeleton className="h-24 w-80" />
      </div>
    );
  if (gate !== "allowed")
    return (
      <div className="admin-gate">
        <Notice>
          {gate === "denied"
            ? "你的账号没有管理后台权限，请联系管理员分配角色。"
            : message}
        </Notice>
        <a href="/account">个人中心</a>
        <a href="/admin/login">重新登录后台</a>
      </div>
    );
  return <AdminConsole user={currentUser} />;
}
function AdminConsole({ user }: { user?: AuthUser }) {
  const publicEdition = useEdition().edition === "public";
  const connection = useConnection(),
    errors = useErrors();
  const [menu, setMenu] = useState(false);
  useEffect(() => {
    admin.connect();
    return admin.disconnect;
  }, []);
  return (
    <BrowserRouter basename="/admin">
      <div className="app-shell">
        <aside
          id="admin-sidebar"
          className={menu ? "sidebar expanded" : "sidebar"}
        >
          <NavLink to="/overview" className="brand">
            <span className="logo-mark">
              <CircuitBoardIcon size={17} />
            </span>
            <span>
              <strong>ostool</strong>
              <small>BOARD PLATFORM</small>
            </span>
          </NavLink>
          <nav aria-label="主导航">
            <SidebarGroup
              label="仪表盘"
              icon={<LayoutDashboardIcon />}
              paths={["/overview", "/jobs", "/operations"]}
            >
              {[
                ["/overview", "运行状态"],
                ["/jobs", "后台任务"],
                ["/operations", "运营状态"],
              ].map(([path, label]) => (
                <NavLink key={path} to={path} onClick={() => setMenu(false)}>
                  <span className="admin-nav-dot" aria-hidden="true" />
                  <span>{label}</span>
                </NavLink>
              ))}
            </SidebarGroup>
            <SidebarGroup
              label="板卡管理"
              icon={<CircuitBoardIcon />}
              paths={["/boards", "/dtbs", "/sessions"]}
            >
              {boardNavigation.map(([path, label]) => (
                <NavLink
                  key={path}
                  to={`/${path}`}
                  onClick={() => setMenu(false)}
                >
                  <span className="admin-nav-dot" aria-hidden="true" />
                  <span>{label}</span>
                </NavLink>
              ))}
            </SidebarGroup>
            {publicEdition && (
              <>
                <SidebarGroup
                  label="租赁管理"
                  icon={<CalendarClockIcon />}
                  paths={["/rentals", "/rental-requests"]}
                >
                  <NavLink to="/rentals" onClick={() => setMenu(false)}>
                    <span className="admin-nav-dot" aria-hidden="true" />
                    <span>租赁列表</span>
                  </NavLink>
                  <NavLink to="/rental-requests" onClick={() => setMenu(false)}>
                    <span className="admin-nav-dot" aria-hidden="true" />
                    <span>租赁申请</span>
                  </NavLink>
                </SidebarGroup>
                <SidebarGroup
                  label="用户管理"
                  icon={<UserCircleIcon />}
                  paths={["/users"]}
                >
                  {userNavigation.map(([path, label]) => (
                    <NavLink
                      end={path === "users"}
                      key={path}
                      to={`/${path}`}
                      onClick={() => setMenu(false)}
                    >
                      <span className="admin-nav-dot" aria-hidden="true" />
                      <span>{label}</span>
                    </NavLink>
                  ))}
                </SidebarGroup>
              </>
            )}
            <NavLink to="/audits" onClick={() => setMenu(false)}>
              <ScrollTextIcon />
              审计日志
            </NavLink>
          </nav>
          <nav className="admin-bottom-nav" aria-label="系统导航">
            <NavLink to="/settings" onClick={() => setMenu(false)}>
              <SettingsIcon />
              系统设置
            </NavLink>
          </nav>
        </aside>
        <main>
          <AdminTopbar
            user={user}
            menuOpen={menu}
            onMenuToggle={() => setMenu((value) => !value)}
          />
          <div className="admin-notices" aria-live="polite">
            {connection === "reconnecting" && (
              <Notice>
                连接暂时中断，当前内容和输入已保留，恢复后自动同步。
              </Notice>
            )}
            {Object.entries(errors).map(([topic, error]) => (
              <Notice key={topic}>
                {topic}：{error}
              </Notice>
            ))}
          </div>
          <div className="admin-workspace">
            <Suspense fallback={<Skeleton className="h-48 w-full" />}>
              <Routes>
                <Route path="/" element={<Navigate replace to="/overview" />} />
                <Route path="/overview" element={<Overview />} />
                <Route path="/jobs" element={<Overview view="jobs" />} />
                <Route
                  path="/operations"
                  element={<Overview view="operations" />}
                />
                <Route path="/audits" element={<Audits />} />
                <Route
                  path="/dashboard"
                  element={<Navigate replace to="/overview" />}
                />
                <Route path="/boards" element={<Boards />} />
                <Route path="/boards/new" element={<BoardEditor />} />
                <Route path="/boards/:boardId" element={<BoardEditor />} />
                <Route path="/dtbs" element={<Dtbs />} />
                <Route path="/sessions" element={<Sessions />} />
                <Route
                  path="/tftp"
                  element={<Navigate replace to="/settings?tab=tftp" />}
                />
                <Route
                  path="/server"
                  element={<Navigate replace to="/settings?tab=server" />}
                />
                <Route path="/settings" element={<Settings />} />
                <Route
                  path="/users"
                  element={
                    publicEdition ? (
                      <Users />
                    ) : (
                      <Navigate replace to="/overview" />
                    )
                  }
                />
                <Route
                  path="/users/roles"
                  element={
                    publicEdition ? (
                      <Roles />
                    ) : (
                      <Navigate replace to="/overview" />
                    )
                  }
                />
                <Route
                  path="/account"
                  element={
                    publicEdition ? (
                      <AccountRedirect />
                    ) : (
                      <Navigate replace to="/overview" />
                    )
                  }
                />
                <Route
                  path="/rental-requests"
                  element={
                    publicEdition ? (
                      <RentalRequests />
                    ) : (
                      <Navigate replace to="/overview" />
                    )
                  }
                />
                <Route
                  path="/rentals/new"
                  element={
                    publicEdition ? (
                      <RentalEditor />
                    ) : (
                      <Navigate replace to="/overview" />
                    )
                  }
                />
                <Route
                  path="/rentals/:rentalId"
                  element={
                    publicEdition ? (
                      <RentalEditor />
                    ) : (
                      <Navigate replace to="/overview" />
                    )
                  }
                />
                <Route
                  path="/rentals"
                  element={
                    publicEdition ? (
                      <Rentals />
                    ) : (
                      <Navigate replace to="/overview" />
                    )
                  }
                />
                <Route
                  path="*"
                  element={<Notice>页面不存在。请选择左侧管理模块。</Notice>}
                />
              </Routes>
            </Suspense>
          </div>
        </main>
        <Toaster position="top-right" duration={2500} richColors closeButton />
      </div>
    </BrowserRouter>
  );
}

function SidebarGroup({
  label,
  icon,
  paths,
  children,
}: {
  label: string;
  icon: ReactNode;
  paths: string[];
  children: ReactNode;
}) {
  const { pathname } = useLocation();
  const active = paths.some(
    (path) => pathname === path || pathname.startsWith(`${path}/`),
  );
  return (
    <details className="admin-nav-group" open>
      <summary className={active ? "active" : undefined}>
        {icon}
        <span>{label}</span>
        <span className="admin-nav-caret" aria-hidden="true">
          <ChevronRightIcon size={18} />
        </span>
      </summary>
      <div>{children}</div>
    </details>
  );
}

function AccountRedirect() {
  useEffect(() => window.location.replace("/account"), []);
  return <Notice>正在打开个人中心…</Notice>;
}

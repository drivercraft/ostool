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
} from "lucide-react";
import { EditionProvider, useEdition } from "./edition";
import { api, type AuthUser } from "@/api/client";
import { ThemeProvider } from "next-themes";
import { AdminTopbar } from "@/components/admin-topbar";
import { admin, useConnection, useErrors } from "@/api/events";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Toaster } from "@/components/ui/sonner";
import { Notice } from "@/components/forms";
const Boards = lazy(() => import("./pages/Boards"));
const BoardEditor = lazy(() => import("./pages/BoardEditor"));
const Overview = lazy(() => import("./pages/Overview"));
const Dtbs = lazy(() => import("./pages/Dtbs"));
const Sessions = lazy(() => import("./pages/Sessions"));
const Settings = lazy(() => import("./pages/SystemSettings"));
const Rentals = lazy(() => import("./pages/Rentals"));
const Users = lazy(() => import("./pages/Users"));
const Roles = lazy(() => import("./pages/Roles"));
const Account = lazy(() => import("./pages/Account"));
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
  const [gate, setGate] = useState<"checking" | "allowed" | "denied" | "error">(
    "checking",
  );
  const [message, setMessage] = useState("");
  const [currentUser, setCurrentUser] = useState<AuthUser>();
  useEffect(() => {
    let active = true;
    (async () => {
      if (config.edition === "local") {
        if (active) setGate("allowed");
        return;
      }
      if (!localStorage.getItem("ostool.access_token")) {
        window.location.assign("/login");
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
  }, [config]);
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
        <a href="/login">重新登录</a>
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
            <NavLink to="/overview" onClick={() => setMenu(false)}>
              <LayoutDashboardIcon />
              总览
            </NavLink>
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
                  paths={["/rentals"]}
                >
                  <NavLink to="/rentals" onClick={() => setMenu(false)}>
                    <span className="admin-nav-dot" aria-hidden="true" />
                    <span>租赁列表</span>
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
          </nav>
          <nav className="admin-bottom-nav" aria-label="系统导航">
            <NavLink to="/settings" onClick={() => setMenu(false)}>
              <SettingsIcon />
              系统设置
            </NavLink>
          </nav>
          <div className="connection" role="status">
            <span
              className={
                connection === "connected"
                  ? "connection-dot connected"
                  : "connection-dot"
              }
            />
            {connection === "connected"
              ? "实时已连接"
              : connection === "connecting"
                ? "正在连接…"
                : "连接中断，正在重连…"}
            {connection !== "connected" && (
              <Button variant="ghost" size="sm" onClick={admin.restart}>
                重连
              </Button>
            )}
          </div>
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
                      <Account />
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

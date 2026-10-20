import { lazy, Suspense, useEffect, useState } from "react";
import {
  BrowserRouter,
  NavLink,
  Navigate,
  Route,
  Routes,
} from "react-router-dom";
import {
  LayoutDashboardIcon,
  CircuitBoardIcon,
  FileIcon,
  ClockIcon,
  CalendarClockIcon,
  SettingsIcon,
  MenuIcon,
  UserCircleIcon,
  ExternalLinkIcon,
} from "lucide-react";
import { EditionProvider, useEdition } from "./edition";
import { api } from "@/api/client";
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
  ["boards", "开发板列表", CircuitBoardIcon],
  ["dtbs", "DTB 管理", FileIcon],
  ["sessions", "会话租约", ClockIcon],
] as const;
const userNavigation = [
  ["users", "用户列表", UserCircleIcon],
  ["users/roles", "用户角色", SettingsIcon],
] as const;
export default function App() {
  return (
    <EditionProvider>
      <AdminEntry />
    </EditionProvider>
  );
}
function AdminEntry() {
  const config = useEdition();
  const [gate, setGate] = useState<"checking" | "allowed" | "denied" | "error">(
    "checking",
  );
  const [message, setMessage] = useState("");
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
      if (active) setGate(user.is_admin ? "allowed" : "denied");
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
  return <AdminConsole />;
}
function AdminConsole() {
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
        <header className="mobile-header">
          <strong>ostool</strong>
          <Button
            variant="ghost"
            aria-label="切换导航"
            onClick={() => setMenu((v) => !v)}
          >
            <MenuIcon />
          </Button>
        </header>
        <aside className={menu ? "sidebar expanded" : "sidebar"}>
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
            <details className="admin-nav-group" open>
              <summary>
                <CircuitBoardIcon />
                板卡管理
              </summary>
              <div>
                {boardNavigation.map(([path, label, Icon]) => (
                  <NavLink
                    key={path}
                    to={`/${path}`}
                    onClick={() => setMenu(false)}
                  >
                    <Icon />
                    {label}
                  </NavLink>
                ))}
              </div>
            </details>
            {publicEdition && (
              <>
                <details className="admin-nav-group" open>
                  <summary>
                    <CalendarClockIcon />
                    租赁管理
                  </summary>
                  <div>
                    <NavLink to="/rentals" onClick={() => setMenu(false)}>
                      <CalendarClockIcon />
                      租赁列表
                    </NavLink>
                  </div>
                </details>
                <details className="admin-nav-group" open>
                  <summary>
                    <UserCircleIcon />
                    用户管理
                  </summary>
                  <div>
                    {userNavigation.map(([path, label, Icon]) => (
                      <NavLink
                        end={path === "users"}
                        key={path}
                        to={`/${path}`}
                        onClick={() => setMenu(false)}
                      >
                        <Icon />
                        {label}
                      </NavLink>
                    ))}
                  </div>
                </details>
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
          <header className="admin-topbar">
            <div>
              <span className="admin-breadcrumb">Workspace</span>
              <span className="admin-separator">/</span>
              <strong>设备管理控制台</strong>
            </div>
            <div className="admin-top-actions">
              <a href="/" className="admin-site-link">
                访问网站 <ExternalLinkIcon size={14} />
              </a>
              {publicEdition ? (
                <NavLink className="admin-user-chip" to="/account">
                  <UserCircleIcon size={17} />
                  <span>我的账号</span>
                </NavLink>
              ) : (
                <span className="admin-user-chip">本地版 · 免登录</span>
              )}
            </div>
          </header>
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
        </main>
        <Toaster
          theme="light"
          position="top-right"
          duration={2500}
          richColors
          closeButton
        />
      </div>
    </BrowserRouter>
  );
}

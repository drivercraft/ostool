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
  NetworkIcon,
  SettingsIcon,
  MenuIcon,
  UserCircleIcon,
  ExternalLinkIcon,
} from "lucide-react";
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
const Tftp = lazy(() =>
  import("./pages/Settings").then((m) => ({ default: m.Tftp })),
);
const Server = lazy(() =>
  import("./pages/Settings").then((m) => ({ default: m.Server })),
);
const Account = lazy(() => import("./pages/Account"));
const navigation = [
  ["overview", "总览", LayoutDashboardIcon],
  ["boards", "开发板", CircuitBoardIcon],
  ["dtbs", "DTB", FileIcon],
  ["sessions", "会话租约", ClockIcon],
  ["tftp", "TFTP", NetworkIcon],
  ["server", "Server 配置", SettingsIcon],
  ["account", "账号", UserCircleIcon],
] as const;
export default function App() {
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
            <span className="logo-mark"><CircuitBoardIcon size={17} /></span>
            <span><strong>ostool</strong><small>BOARD PLATFORM</small></span>
          </NavLink>
          <nav aria-label="主导航">
            {navigation.map(([path, label, Icon]) => (
              <NavLink
                key={path}
                to={`/${path}`}
                onClick={() => setMenu(false)}
              >
                <Icon aria-hidden="true" />
                {label}
              </NavLink>
            ))}
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
            <div><span className="admin-breadcrumb">Workspace</span><span className="admin-separator">/</span><strong>设备管理控制台</strong></div>
            <div className="admin-top-actions"><a href="/" className="admin-site-link">访问网站 <ExternalLinkIcon size={14} /></a><NavLink className="admin-user-chip" to="/account"><UserCircleIcon size={17} /><span>我的账号</span></NavLink></div>
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
              <Route path="/tftp" element={<Tftp />} />
              <Route path="/server" element={<Server />} />
              <Route path="/account" element={<Account />} />
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

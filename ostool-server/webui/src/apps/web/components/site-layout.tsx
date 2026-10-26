import { useEffect, useState, type ReactNode } from "react";
import { Link, NavLink, useLocation } from "react-router-dom";
import {
  ArrowRight,
  CircuitBoard,
  FileText,
  Github,
  LogOut,
  Menu,
  ShieldCheck,
  UserRound,
  X,
} from "lucide-react";
import { useSiteSettings } from "@/components/site-settings";
import { useEdition } from "@/edition";
import { useSiteSession } from "@/apps/web/session";
import { ThemeMenu } from "@/components/theme-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
} from "@/components/ui/dropdown-menu";
import { api } from "@/api/client";
const publicNavigation = [
  ["/", "首页"],
  ["/devices", "设备"],
  ["/docs", "文档"],
  ["/about", "关于 ostool"],
] as const;
export function Logo({ light = false }: { light?: boolean }) {
  const { settings } = useSiteSettings();
  return (
    <Link className={`site-logo${light ? " site-logo-light" : ""}`} to="/">
      <span className="logo-mark">
        {settings.logo_url ? (
          <img src={settings.logo_url} alt="" />
        ) : (
          <CircuitBoard size={18} strokeWidth={2.5} />
        )}
      </span>
      <span className="site-logo-copy">
        <strong>{settings.short_name}</strong>
        <small>{settings.tagline}</small>
      </span>
    </Link>
  );
}

function SiteHeader() {
  const { settings } = useSiteSettings();
  const edition = useEdition();
  const publicEdition = edition.edition === "public";
  const [open, setOpen] = useState(false);
  const location = useLocation();
  useEffect(() => {
    setOpen(false);
    const title = location.pathname.startsWith("/account")
      ? "个人工作台"
      : location.pathname === "/login"
        ? "登录"
        : location.pathname === "/register"
          ? "注册"
          : (publicNavigation.find(
              ([path]) => path === location.pathname,
            )?.[1] ?? "开发板协作平台");
    document.title = title + " · " + settings.name;
  }, [location.pathname, settings.name]);
  const { user, checking } = useSiteSession();
  const accountActions = !publicEdition ? (
    <Button asChild size="sm">
      <a href="/admin/overview">打开控制台</a>
    </Button>
  ) : checking ? (
    <Skeleton className="h-9 w-32" aria-label="正在读取登录状态" />
  ) : user ? (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          className="topbar-avatar site-user"
          variant="ghost"
          size="icon"
          aria-label="用户菜单"
          title={user.display_name || user.username || user.email}
        >
          {Array.from(
            user.display_name || user.username || user.email,
          )[0]?.toUpperCase()}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="site-account-menu">
        <DropdownMenuLabel>
          <strong>{user.display_name || "我的账号"}</strong>
          <small>{user.email}</small>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem asChild>
          <Link to="/account">
            <UserRound />
            个人中心
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/account/profile">
            <UserRound />
            基本信息
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/account/security">
            <ShieldCheck />
            账号安全
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/account/rentals">
            <CircuitBoard />
            我的租赁
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/account/requests">
            <FileText />
            租赁申请
          </Link>
        </DropdownMenuItem>
        {(user.can_access_admin ?? user.is_admin) && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem asChild>
              {(!publicEdition ||
                (user?.can_access_admin ?? user?.is_admin)) && (
                <a href="/admin/overview">管理后台</a>
              )}
            </DropdownMenuItem>
          </>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={() =>
            void api.logout().finally(() => window.location.assign("/"))
          }
        >
          <LogOut />
          退出登录
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  ) : (
    <>
      <Button asChild variant="ghost" size="sm">
        <Link to="/login" onClick={() => setOpen(false)}>
          登录
        </Link>
      </Button>
      {edition.allow_registration && (
        <Button asChild size="sm">
          <Link to="/register" onClick={() => setOpen(false)}>
            免费注册
          </Link>
        </Button>
      )}
    </>
  );
  return (
    <header className="site-header">
      <div className="site-header-inner">
        <Logo />
        <nav
          className={open ? "site-nav site-nav-open" : "site-nav"}
          aria-label="网站导航"
        >
          {publicNavigation.map(([href, label]) => (
            <NavLink
              key={href}
              to={href}
              end={href === "/"}
              onClick={() => setOpen(false)}
            >
              {label}
            </NavLink>
          ))}
          {!user && <div className="mobile-nav-actions">{accountActions}</div>}
        </nav>
        <div className="site-right-actions">
          <ThemeMenu />
          <div className={`site-actions${user ? " has-user" : ""}`}>
            {accountActions}
          </div>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="site-menu-toggle"
          aria-label={open ? "关闭菜单" : "打开菜单"}
          onClick={() => setOpen((value) => !value)}
        >
          {open ? <X size={20} /> : <Menu size={20} />}
        </Button>
      </div>
    </header>
  );
}

function SiteFooter() {
  const { settings } = useSiteSettings();
  const publicEdition = useEdition().edition === "public";
  return (
    <footer className="site-footer">
      <div className="site-footer-main">
        <div className="footer-brand">
          <Logo />
          <p>{settings.description}</p>
          {settings.github_url && (
            <a
              className="footer-source"
              href={settings.github_url}
              rel="noopener noreferrer"
            >
              <Github size={18} /> 开源项目 <ArrowRight size={16} />
            </a>
          )}
        </div>
        <nav className="footer-column" aria-label="平台导航">
          <strong>平台</strong>
          <Link to="/">首页</Link>
          <Link to="/devices">设备资源</Link>
          <Link to="/about">关于 ostool</Link>
        </nav>
        <nav className="footer-column" aria-label="开发资源">
          <strong>开发资源</strong>
          <Link to="/docs">快速开始</Link>
          <Link to="/docs?page=cli">命令行工具</Link>
          <Link to="/docs?page=api">API 参考</Link>
        </nav>
        <nav className="footer-column" aria-label="支持与服务">
          <strong>支持与服务</strong>
          {settings.support_email && (
            <a href={"mailto:" + settings.support_email}>
              {settings.support_email}
            </a>
          )}
          {settings.support_phone && (
            <a href={"tel:" + settings.support_phone}>
              {settings.support_phone}
            </a>
          )}
          {settings.support_work_hours && (
            <span>{settings.support_work_hours}</span>
          )}
          {settings.support_url && (
            <a href={settings.support_url} rel="noopener noreferrer">
              联系支持
            </a>
          )}
          {publicEdition && (
            <>
              <Link to="/legal/terms">{settings.terms_title}</Link>
              <Link to="/legal/privacy">{settings.privacy_title}</Link>
            </>
          )}
          {!publicEdition && <a href="/admin/overview">本地控制台</a>}
        </nav>
      </div>
      <div className="site-footer-bottom">
        <div>
          <span>
            © {new Date().getFullYear()} {settings.copyright}
          </span>
          {settings.operator && <span> · {settings.operator}</span>}
          {settings.address && <p>{settings.address}</p>}
        </div>
        <span className="footer-social">
          {settings.icp_number && (
            <a href={settings.icp_url || undefined} rel="noopener noreferrer">
              {settings.icp_number}
            </a>
          )}
          {settings.police_number && (
            <a
              href={settings.police_url || undefined}
              rel="noopener noreferrer"
            >
              {settings.police_number}
            </a>
          )}
        </span>
      </div>
    </footer>
  );
}

export function PublicLayout({ children }: { children: ReactNode }) {
  const { settings } = useSiteSettings();
  return (
    <div className="public-site">
      <SiteHeader />
      {settings.maintenance && (
        <div className="site-maintenance" role="status">
          {settings.maintenance_message}
        </div>
      )}
      {children}
      <SiteFooter />
    </div>
  );
}

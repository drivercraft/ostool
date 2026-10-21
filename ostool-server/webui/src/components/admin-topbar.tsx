import { useMemo, useRef, useState } from "react";
import { Link, useLocation } from "react-router-dom";
import { useTheme } from "next-themes";
import {
  Bell,
  CheckCheck,
  CircuitBoard,
  ExternalLink,
  LogOut,
  Menu,
  Monitor,
  Moon,
  Settings,
  Sun,
  UserRound,
} from "lucide-react";
import { api, type AuthUser } from "@/api/client";
import { useConnection, useErrors, useResource } from "@/api/events";
import { useEdition } from "@/edition";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

const pageTitles: Record<string, string> = {
  "/overview": "总览",
  "/boards": "开发板列表",
  "/boards/new": "新建开发板",
  "/dtbs": "DTB 管理",
  "/sessions": "会话租约",
  "/rentals": "租赁列表",
  "/users": "用户列表",
  "/users/roles": "用户角色",
  "/settings": "系统设置",
  "/account": "我的账号",
};
const topicPaths: Record<string, string> = {
  boards: "/boards",
  quarantined_boards: "/boards",
  runtimes: "/boards",
  sessions: "/sessions",
  loaders: "/boards",
  users: "/users",
  roles: "/users/roles",
  rentals: "/rentals",
  dtbs: "/dtbs",
  server: "/settings",
  tftp: "/settings?tab=tftp",
  tftp_status: "/settings?tab=tftp",
  power_actions: "/boards",
};
const topicLabels: Record<string, string> = {
  boards: "开发板",
  quarantined_boards: "配置隔离",
  runtimes: "板卡运行状态",
  sessions: "会话",
  loaders: "设备发现",
  users: "用户",
  roles: "角色",
  rentals: "租赁",
  dtbs: "DTB",
  server: "服务器配置",
  tftp: "TFTP",
  tftp_status: "TFTP 状态",
  power_actions: "电源操作",
};
interface AdminNotification {
  id: string;
  title: string;
  description: string;
  to: string;
}

type MenuKind = "messages" | "theme" | "account";
interface TopbarMenus {
  active: MenuKind | null;
  change: (kind: MenuKind, open: boolean) => void;
  restoreFocus: (event: Event) => void;
}

function Notifications({ menus }: { menus: TopbarMenus }) {
  const errors = useErrors(),
    connection = useConnection();
  const runtimes = useResource("runtimes"),
    power = useResource("power_actions"),
    quarantined = useResource("quarantined_boards");
  const notices = useMemo(() => {
    const result: AdminNotification[] = [];
    if (connection === "reconnecting")
      result.push({
        id: "connection",
        title: "实时连接中断",
        description: "连接恢复后自动同步，编辑草稿会保留。",
        to: "/overview",
      });
    for (const [topic, error] of Object.entries(errors))
      result.push({
        id: `topic:${topic}`,
        title: `${topicLabels[topic] ?? topic}同步失败`,
        description: error,
        to: topicPaths[topic] ?? "/overview",
      });
    for (const [board, runtime] of Object.entries(runtimes ?? {})) {
      if (runtime.last_release_error)
        result.push({
          id: `release:${board}`,
          title: `${board} 释放失败`,
          description: runtime.last_release_error,
          to: `/boards/${encodeURIComponent(board)}`,
        });
    }
    for (const action of power ?? []) {
      if (action.state === "failed")
        result.push({
          id: `power:${action.id}`,
          title: `${action.action === "on" ? "上电" : "下电"}操作失败`,
          description: action.message ?? "请检查电源配置和设备连接。",
          to: "/boards",
        });
    }
    for (const board of quarantined ?? [])
      result.push({
        id: `quarantine:${board.backup_path}`,
        title: "不兼容板卡配置已隔离",
        description: board.reason,
        to: "/boards",
      });
    return result;
  }, [connection, errors, runtimes, power, quarantined]);
  const [read, setRead] = useState<Record<string, string>>({});
  const unread = notices.filter(
    (notice) => read[notice.id] !== notice.description,
  ).length;
  function markRead() {
    setRead(
      Object.fromEntries(
        notices.map((notice) => [notice.id, notice.description]),
      ),
    );
  }
  return (
    <Popover
      open={menus.active === "messages"}
      onOpenChange={(value) => {
        menus.change("messages", value);
        if (value) markRead();
      }}
    >
      <PopoverTrigger asChild>
        <Button
          className="topbar-icon"
          variant="ghost"
          size="icon"
          aria-label={`消息，${unread} 条未读`}
          title="消息"
        >
          <Bell aria-hidden="true" />
          {unread > 0 && (
            <span
              className="topbar-badge"
              data-testid="notification-badge"
              aria-hidden="true"
            >
              {unread > 99 ? "99+" : unread}
            </span>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        className="notification-menu"
        onCloseAutoFocus={menus.restoreFocus}
        aria-label="消息"
      >
        <div className="notification-heading">
          <strong>消息</strong>
          <Button
            size="sm"
            variant="ghost"
            onClick={markRead}
            disabled={!unread}
          >
            <CheckCheck />
            全部已读
          </Button>
        </div>
        <div className="notification-items">
          {notices.length ? (
            notices.map((notice) => (
              <Link
                key={notice.id}
                to={notice.to}
                className="notification-item"
                onClick={() => menus.change("messages", false)}
              >
                <span className="notification-item-icon">
                  <Bell size={15} />
                </span>
                <span>
                  <strong>{notice.title}</strong>
                  <small>{notice.description}</small>
                </span>
              </Link>
            ))
          ) : (
            <div className="notification-empty">
              <Bell />
              <strong>暂无消息</strong>
              <span>设备与服务运行提醒会在这里显示。</span>
            </div>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}

function ThemeMenu({ menus }: { menus: TopbarMenus }) {
  const { theme, setTheme, resolvedTheme } = useTheme();
  const Icon =
    theme === "system" ? Monitor : resolvedTheme === "dark" ? Moon : Sun;
  return (
    <DropdownMenu
      modal={false}
      open={menus.active === "theme"}
      onOpenChange={(open) => menus.change("theme", open)}
    >
      <DropdownMenuTrigger asChild>
        <Button
          className="topbar-icon"
          variant="ghost"
          size="icon"
          aria-label="切换主题"
          title="切换主题"
        >
          <Icon aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="topbar-dropdown"
        onCloseAutoFocus={menus.restoreFocus}
      >
        <DropdownMenuLabel>主题</DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuRadioGroup
          value={theme ?? "system"}
          onValueChange={setTheme}
        >
          <DropdownMenuRadioItem value="light">
            <Sun className="theme-option-icon" />
            明亮
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="dark">
            <Moon className="theme-option-icon" />
            黑暗
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="system">
            <Monitor className="theme-option-icon" />
            跟随系统
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function AccountMenu({ user, menus }: { user?: AuthUser; menus: TopbarMenus }) {
  const publicEdition = useEdition().edition === "public";
  const name = publicEdition
    ? user?.display_name || user?.email || "管理员"
    : "本地工作区";
  return (
    <DropdownMenu
      modal={false}
      open={menus.active === "account"}
      onOpenChange={(open) => menus.change("account", open)}
    >
      <DropdownMenuTrigger asChild>
        <Button
          className="topbar-avatar"
          variant="ghost"
          size="icon"
          aria-label="账号菜单"
          title={name}
        >
          {publicEdition ? (
            <span>{Array.from(name)[0]?.toUpperCase()}</span>
          ) : (
            <Monitor aria-hidden="true" />
          )}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="topbar-dropdown account-dropdown"
        onCloseAutoFocus={menus.restoreFocus}
      >
        <DropdownMenuLabel className="account-menu-card">
          <span className="account-menu-avatar">
            {publicEdition ? <UserRound /> : <Monitor />}
          </span>
          <span>
            <strong>{name}</strong>
            <small>{publicEdition ? user?.email : "本地版 · 免登录"}</small>
          </span>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        {publicEdition && (
          <DropdownMenuItem asChild>
            <Link to="/account">
              <UserRound />
              我的账号
            </Link>
          </DropdownMenuItem>
        )}
        <DropdownMenuItem asChild>
          <Link to="/settings">
            <Settings />
            系统设置
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <a href="/">
            <ExternalLink />
            访问网站
          </a>
        </DropdownMenuItem>
        {publicEdition && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="text-destructive"
              onSelect={() => {
                void api
                  .logout()
                  .catch(() => {})
                  .finally(() => window.location.assign("/login"));
              }}
            >
              <LogOut />
              退出登录
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function AdminTopbar({
  user,
  menuOpen,
  onMenuToggle,
}: {
  user?: AuthUser;
  menuOpen: boolean;
  onMenuToggle: () => void;
}) {
  const [activeMenu, setActiveMenu] = useState<MenuKind | null>(null);
  const currentMenu = useRef<MenuKind | null>(null);
  const menus: TopbarMenus = {
    active: activeMenu,
    change(kind, open) {
      if (open) {
        currentMenu.current = kind;
        setActiveMenu(kind);
      } else if (currentMenu.current === kind) {
        currentMenu.current = null;
        setActiveMenu(null);
      }
    },
    // An exiting menu must not steal focus from the next menu in the header.
    // Escape still restores the original trigger when no other menu is open.
    restoreFocus(event) {
      if (currentMenu.current !== null) event.preventDefault();
    },
  };
  const { pathname } = useLocation();
  const title =
    pageTitles[pathname] ??
    (pathname.startsWith("/boards/") ? "编辑开发板" : "设备管理控制台");
  return (
    <header className="admin-topbar">
      <div className="topbar-title-area">
        <Button
          className="topbar-nav-toggle"
          variant="ghost"
          size="icon"
          aria-label="切换导航"
          aria-expanded={menuOpen}
          aria-controls="admin-sidebar"
          onClick={onMenuToggle}
        >
          <Menu />
        </Button>
        <Link className="topbar-mobile-brand" to="/overview">
          <span className="logo-mark">
            <CircuitBoard size={17} />
          </span>
          <strong>ostool</strong>
        </Link>
        <h1 className="topbar-page-title">{title}</h1>
      </div>
      <div className="admin-top-actions">
        <Notifications menus={menus} />
        <ThemeMenu menus={menus} />
        <AccountMenu user={user} menus={menus} />
      </div>
    </header>
  );
}

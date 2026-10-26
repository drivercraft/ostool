import {
  SiteSettingsProvider,
  useSiteSettings,
} from "@/components/site-settings";
import { AuthLegal } from "@/components/auth-legal";
import { Checkbox } from "@/components/ui/checkbox";
import { FormEvent, useEffect, useState } from "react";
import {
  ArrowRight,
  CheckCircle2,
  ChevronDown,
  LogOut,
  UserRound,
  CircuitBoard,
  Command,
  FileText,
  Github,
  Menu,
  Radio,
  ShieldCheck,
  Terminal,
  X,
  Zap,
} from "lucide-react";
import {
  BrowserRouter,
  Link,
  NavLink,
  Navigate,
  Route,
  Routes,
  useLocation,
  useNavigate,
} from "react-router-dom";
import { EditionProvider, useEdition } from "@/edition";
import MemberWorkspace from "@/apps/web/pages/MemberWorkspace";
import { BoardVisual } from "@/apps/web/components/board-visual";
import { Documentation } from "@/apps/web/pages/Documentation";
import { Skeleton } from "@/components/ui/skeleton";
import { DeviceCatalog } from "@/apps/web/pages/DeviceCatalog";
import { SiteSessionProvider, useSiteSession } from "@/apps/web/session";
import { ThemeMenu } from "@/components/theme-menu";
import { ThemeProvider } from "next-themes";
import { Toaster } from "@/components/ui/sonner";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
} from "@/components/ui/dropdown-menu";
import { TextField } from "@/components/forms";
import { useCountdown } from "@/components/security-hooks";
import { HumanVerification } from "@/components/human-verification";
import { PasswordReset } from "@/components/password-reset";
import { SecurityVerification } from "@/components/security-verification";
import {
  api,
  notifyAuthChanged,
  type HumanProof,
  type EmailChallenge,
} from "@/api/client";
import { Button } from "@/components/ui/button";

const publicNavigation = [
  ["/", "首页"],
  ["/devices", "设备"],
  ["/docs", "文档"],
  ["/about", "关于 ostool"],
] as const;

function Logo({ light = false }: { light?: boolean }) {
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
        <button className="site-user" aria-label="用户菜单">
          <span className="site-avatar">
            {(user.display_name || user.email).slice(0, 1).toUpperCase()}
          </span>
          <span className="site-user-copy">
            <strong>{user.display_name || user.email}</strong>
            <small>个人工作台</small>
          </span>
          <ChevronDown size={14} />
        </button>
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
          <Link to="/account/profile">基本信息</Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/account/security">账号安全</Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/account/rentals">我的租赁</Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/account/requests">租赁申请</Link>
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
      <Link className="site-login" to="/login" onClick={() => setOpen(false)}>
        登录
      </Link>
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
          <div className="mobile-nav-actions">{accountActions}</div>
        </nav>
        <div className="site-right-actions">
          <ThemeMenu />
          <div className="site-actions">{accountActions}</div>
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
  const { user } = useSiteSession();
  const publicEdition = useEdition().edition === "public";
  return (
    <footer className="site-footer">
      <div className="site-footer-main">
        <div className="footer-brand">
          <Logo light />
          <p>{settings.description}</p>
        </div>
        <div className="footer-column">
          <strong>产品</strong>
          <Link to="/devices">设备管理</Link>
          <Link to="/docs">文档中心</Link>
          {(!publicEdition || (user?.can_access_admin ?? user?.is_admin)) && (
            <a href="/admin/overview">管理后台</a>
          )}
        </div>
        <div className="footer-column">
          <strong>资源</strong>
          <Link to="/docs?page=cli">ostool CLI</Link>
          <Link to="/docs?page=api">API 参考</Link>
          {settings.github_url && (
            <a href={settings.github_url} rel="noopener noreferrer">
              GitHub
            </a>
          )}
          {publicEdition && (
            <>
              <Link to="/legal/terms">{settings.terms_title}</Link>
              <Link to="/legal/privacy">{settings.privacy_title}</Link>
            </>
          )}
          {settings.support_email && (
            <a href={"mailto:" + settings.support_email}>
              {settings.support_email}
            </a>
          )}
          {settings.support_phone && <span>{settings.support_phone}</span>}
          {settings.support_work_hours && (
            <span>{settings.support_work_hours}</span>
          )}
          {settings.support_url && <a href={settings.support_url}>联系支持</a>}
        </div>
        <div className="footer-column">
          <strong>开始使用</strong>
          <span>连接你的第一块开发板</span>
          <Button asChild variant="outline" size="sm">
            <a href={publicEdition ? "/account" : "/admin/overview"}>
              {publicEdition ? "个人工作台" : "进入控制台"}{" "}
              <ArrowRight size={14} />
            </a>
          </Button>
        </div>
      </div>
      <div className="site-footer-bottom">
        <span>
          © {new Date().getFullYear()} {settings.copyright}
          {settings.operator && <span> · {settings.operator}</span>}
          {settings.address && <span> · {settings.address}</span>}
        </span>
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

function PublicLayout({ children }: { children: React.ReactNode }) {
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

function Home() {
  const publicEdition = useEdition().edition === "public";
  const { user } = useSiteSession();
  const startPath = publicEdition
    ? user
      ? "/account"
      : "/register"
    : "/admin/overview";
  return (
    <PublicLayout>
      <main className="public-home">
        <section className="public-hero-section">
          <div className="hero-glow hero-glow-one" />
          <div className="hero-glow hero-glow-two" />
          <div className="hero-copy">
            <div className="hero-kicker">
              <span className="status-pulse" /> 开发板协作平台{" "}
              <span className="hero-kicker-divider" /> OPEN SOURCE
            </div>
            <h1>
              连接每一块开发板，<em>专注每一次创新。</em>
            </h1>
            <p className="hero-subtitle">
              从构建、启动到串口调试，ostool 将真实开发板、QEMU
              与团队工作流连在一起。 在浏览器管理资源，在终端完成开发。
            </p>
            <div className="hero-actions">
              <Button asChild size="lg">
                <a href={startPath}>
                  开始使用 <ArrowRight size={16} />
                </a>
              </Button>
              <Button asChild size="lg" variant="outline">
                <Link to="/docs">
                  查看文档 <FileText size={16} />
                </Link>
              </Button>
            </div>
            <div className="hero-trust">
              <span>
                <CheckCircle2 size={15} /> 开源工具链
              </span>
              <span>
                <ShieldCheck size={15} /> 私有化部署
              </span>
              <span>
                <Zap size={15} /> 实时状态同步
              </span>
            </div>
          </div>
          <div className="hero-product">
            <BoardVisual />
            <span className="hero-visual-note">设备工作流示意</span>
          </div>
        </section>
        <section className="logo-strip">
          <span>为硬件团队提供稳定的日常基础设施</span>
          <div>
            <b>UART</b>
            <b>QEMU</b>
            <b>U-BOOT</b>
            <b>OTA</b>
            <b>PXE</b>
          </div>
        </section>
        <section className="public-section feature-section">
          <div className="section-heading">
            <span className="section-label">ONE WORKSPACE</span>
            <h2>硬件开发，也可以很从容。</h2>
            <p>
              {publicEdition
                ? "把设备、文件、会话和权限放在同一个上下文里，让团队更快地发现问题、复现问题和交付结果。"
                : "把设备配置、文件投递与串口调试放在同一个工作区，专注本地构建、启动和系统验证。"}
            </p>
          </div>
          <div className="feature-grid">
            <FeatureCard
              icon={<CircuitBoard />}
              number="01"
              title="设备资产"
              text="统一管理实体开发板与 QEMU 虚拟设备，实时了解在线状态、固件和当前占用者。"
            />
            <FeatureCard
              icon={<Radio />}
              number="02"
              title="实时会话"
              text="串口、TFTP、HTTP Boot 和电源控制协同工作，租约到期后自动释放资源。"
            />
            <FeatureCard
              icon={<Command />}
              number="03"
              title="CLI 优先"
              text={
                publicEdition
                  ? "浏览器和 ostool CLI 使用同一套登录与 API 契约，脚本和人工操作自然衔接。"
                  : "浏览器与 ostool CLI 直连本地服务，脚本和人工操作自然衔接。"
              }
            />
          </div>
        </section>
        <section className="public-section split-section">
          <div className="split-visual">
            <div className="terminal-card">
              <div className="terminal-top">
                <span>
                  <i />
                  <i />
                  <i />
                </span>
                <small>ostool terminal</small>
              </div>
              <div className="terminal-lines">
                {publicEdition ? (
                  <>
                    <p>
                      <b>$</b> ostool login
                    </p>
                    <p className="dim">Opening device authorization page...</p>
                    <p>
                      <b>✓</b> Authenticated as <strong>team@acme.dev</strong>
                    </p>
                  </>
                ) : (
                  <p className="dim">Connected to local board server</p>
                )}
                <p>
                  <b>$</b> ostool board run --board-type rk3568
                </p>
                <p className="blue">● lease acquired · serial connected</p>
              </div>
            </div>
          </div>
          <div className="split-copy">
            <span className="section-label">MADE FOR BUILDERS</span>
            <h2>从终端到浏览器，保持同一套体验。</h2>
            <p>
              开发、测试和运维不需要在多个工具之间来回切换。ostool 用清晰的
              API、可追踪的事件和安全的会话，把硬件访问变成团队基础设施。
            </p>
            <ul>
              <li>
                <CheckCircle2 />
                设备租约避免资源冲突
              </li>
              <li>
                <CheckCircle2 />
                SSE 实时推送，无需手动刷新
              </li>
              <li>
                <CheckCircle2 />
                {publicEdition ? "OAuth 登录与个人访问令牌" : "本地设备直连"}
              </li>
            </ul>
            <Link className="text-arrow-link" to="/docs">
              了解工作方式 <ArrowRight size={15} />
            </Link>
          </div>
        </section>
        <section className="public-cta">
          <div>
            <span className="section-label">READY WHEN YOU ARE</span>
            <h2>从第一块板卡开始，建立你的硬件工作台。</h2>
            <p>几分钟完成部署，随后专注于真正重要的开发工作。</p>
          </div>
          <Button asChild size="lg">
            <a href={startPath}>
              {publicEdition
                ? user
                  ? "打开个人工作台"
                  : "创建账号"
                : "打开本地控制台"}{" "}
              <ArrowRight size={16} />
            </a>
          </Button>
        </section>
      </main>
    </PublicLayout>
  );
}

function FeatureCard({
  icon,
  number,
  title,
  text,
}: {
  icon: React.ReactNode;
  number: string;
  title: string;
  text: string;
}) {
  return (
    <article className="feature-card">
      <div className="feature-card-top">
        <span className="feature-icon">{icon}</span>
        <span className="feature-number">{number}</span>
      </div>
      <h3>{title}</h3>
      <p>{text}</p>
      <ArrowRight className="feature-arrow" size={17} />
    </article>
  );
}

function Devices() {
  const publicEdition = useEdition().edition === "public";
  return (
    <PublicLayout>
      <main className="public-inner-page devices-page">
        <section className="device-product-hero">
          <div className="inner-hero">
            <span className="section-label">YOUR HARDWARE. ONE WORKSPACE.</span>
            <h1>
              让硬件资源，
              <br />
              <em>成为研发的助力。</em>
            </h1>
            <p>
              从一块开发板到整个实验室，统一管理设备配置、启动方式与调试入口。用清晰的资源目录，连接每一次构建与验证。
            </p>
            <div className="hero-actions">
              <Button asChild>
                <a href="#device-catalog">
                  浏览设备 <ArrowRight size={16} />
                </a>
              </Button>
              <Button asChild variant="outline">
                <Link to="/docs?page=devices">设备使用指南</Link>
              </Button>
            </div>
            <div className="device-capabilities">
              <span>
                <CircuitBoard /> 实体与虚拟设备
              </span>
              <span>
                <Terminal /> 串口与命令行
              </span>
              <span>
                <ShieldCheck />{" "}
                {publicEdition ? "按授权时段使用" : "本地免登录直连"}
              </span>
            </div>
          </div>
          <BoardVisual label="DEVICE PLATFORM" compact />
        </section>
        <section className="device-value-strip" aria-label="设备平台能力">
          <div>
            <span>01 / CONNECT</span>
            <h2>发现与配置</h2>
            <p>集中维护板型、启动配置和设备标识。</p>
          </div>
          <div>
            <span>02 / WORK</span>
            <h2>开发与调试</h2>
            <p>通过 CLI 连接串口、投递固件和启动系统。</p>
          </div>
          <div>
            <span>03 / COLLABORATE</span>
            <h2>{publicEdition ? "时段与协作" : "本地工作流"}</h2>
            <p>
              {publicEdition
                ? "查看使用日历，按需申请设备使用时间。"
                : "直接连接本地控制器，管理设备与短期会话。"}
            </p>
          </div>
        </section>
        <DeviceCatalog />
        <div className="inner-callout">
          <div>
            <span className="section-label">BUILD WHAT COMES NEXT</span>
            <h2>开始你的下一次硬件验证。</h2>
            <p>
              {publicEdition
                ? "进入个人工作台，查看租赁并申请设备使用时段。"
                : "打开本地控制台，配置你的第一块开发板。"}
            </p>
          </div>
          <Button asChild>
            <a href={publicEdition ? "/account" : "/admin/overview"}>
              {publicEdition ? "个人工作台" : "进入控制台"}
              <ArrowRight size={16} />
            </a>
          </Button>
        </div>
      </main>
    </PublicLayout>
  );
}
function Docs() {
  return (
    <PublicLayout>
      <Documentation />
    </PublicLayout>
  );
}

function About() {
  return (
    <PublicLayout>
      <main className="public-inner-page about-page">
        <div className="inner-hero">
          <span className="section-label">OPEN SOURCE</span>
          <h1>为真实硬件而生的工具。</h1>
          <p>
            ostool
            是一个面向嵌入式开发团队的开源工具链，连接构建、启动和设备管理的每一个环节。
          </p>
          <Button asChild>
            <a href="https://github.com/drivercraft/ostool">
              在 GitHub 上查看 <Github size={15} />
            </a>
          </Button>
        </div>
      </main>
    </PublicLayout>
  );
}

function Login() {
  const navigate = useNavigate();
  const location = useLocation();
  const query = new URLSearchParams(location.search);
  const userCode = query.get("user_code") ?? "";
  const [deviceConfirmed, setDeviceConfirmed] = useState(false);
  useEffect(() => setDeviceConfirmed(false), [userCode]);
  const registering =
    location.pathname === "/register" || query.get("register") === "1";
  const [confirmPassword, setConfirmPassword] = useState("");
  const authConfig = useEdition();
  const legalSite = useSiteSettings();
  useEffect(() => {
    document.title =
      (userCode ? "批准 CLI 登录" : registering ? "注册" : "登录") +
      " · " +
      legalSite.settings.name;
  }, [registering, userCode, legalSite.settings.name]);
  const [accepted, setAccepted] = useState(false);
  useEffect(
    () => setAccepted(false),
    [legalSite.terms_version, legalSite.privacy_version],
  );

  const registrationAllowed = authConfig.allow_registration;
  const emailRequired =
    registering && authConfig.registration_email_verification;
  const [proof, setProof] = useState<HumanProof>();
  const [refreshKey, setRefreshKey] = useState(0);
  const [challenge, setChallenge] = useState<string>();
  const [otp, setOtp] = useState("");
  const [emailChallenge, setEmailChallenge] = useState<EmailChallenge>();
  const [emailCode, setEmailCode] = useState("");
  const [emailUntil, setEmailUntil] = useState(0);
  const resendRemaining = useCountdown(emailUntil);
  const [username, setUsername] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (userCode && !deviceConfirmed) {
      setMessage("请确认由您发起登录，并核对终端验证码。");
      return;
    }
    if (registering && password !== confirmPassword) {
      setMessage("两次输入的密码不一致");
      return;
    }
    setBusy(true);
    setMessage("");
    try {
      if (
        registering &&
        (Array.from(password).length < 15 ||
          new TextEncoder().encode(password).length > 1024)
      )
        throw new Error("密码需要至少 15 个字符，且不超过 1024 字节");
      const result = challenge
        ? await api.verifyMfa(challenge, otp)
        : registering
          ? await api.register(
              username,
              email,
              password,
              displayName,
              proof ?? {},
              emailChallenge
                ? { id: emailChallenge.challenge_id, code: emailCode }
                : undefined,
              {
                accepted,
                terms_version: legalSite.terms_version,
                privacy_version: legalSite.privacy_version,
              },
            )
          : await api.login(email, password, proof ?? {});
      if (result.mfa_required) {
        setChallenge(result.challenge_id);
        setPassword("");
        setOtp("");
        return;
      }
      notifyAuthChanged();
      if (userCode) {
        await api.authorizeDevice(userCode);
        setMessage("设备登录已批准，可以返回 ostool CLI。");
      } else {
        const redirect = query.get("redirect");
        navigate(
          redirect?.startsWith("/account") && !redirect.startsWith("//")
            ? redirect
            : "/account",
        );
      }
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "登录失败");
      if (!challenge) setRefreshKey((v) => v + 1);
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="auth-page auth-page-fullscreen">
      <main className="auth-shell">
        <aside className="auth-aside" aria-label="平台介绍">
          <Logo light />
          <div className="auth-aside-copy">
            <span className="section-label">YOUR HARDWARE WORKSPACE</span>
            <h1>
              连接每一块开发板，
              <br />
              <em>专注下一次创造。</em>
            </h1>
            <p>
              从设备发现到远程调试，让开发资源与团队协作在同一个工作区有序连接。
            </p>
            <div className="auth-aside-points">
              <div>
                <CircuitBoard />
                <span>
                  <strong>设备，一目了然</strong>
                  <small>浏览开发板，找到适合项目的设备。</small>
                </span>
              </div>
              <div>
                <CheckCircle2 />
                <span>
                  <strong>时段，按需安排</strong>
                  <small>查看空闲日历，申请专属使用时段。</small>
                </span>
              </div>
              <div>
                <ShieldCheck />
                <span>
                  <strong>访问，安全有序</strong>
                  <small>管理账号与 CLI 凭据，保持工作连续。</small>
                </span>
              </div>
            </div>
          </div>
          <div className="auth-aside-footer">
            <span>{legalSite.settings.short_name} · 开发板协作平台</span>
            <Link to="/docs">
              阅读文档 <ArrowRight size={16} />
            </Link>
          </div>
        </aside>
        <section
          className="auth-form-pane"
          aria-label={registering ? "注册账号" : "用户登录"}
        >
          <div className="auth-page-tools">
            <span className="auth-mobile-brand">
              <Logo />
            </span>
            <Link to="/">返回首页</Link>
            <ThemeMenu />
          </div>
          <div className="auth-form-scroll">
            <form className="auth-card" onSubmit={submit}>
              <div>
                <h2>
                  {userCode
                    ? "批准 CLI 登录"
                    : registering
                      ? "创建你的账号"
                      : "欢迎回来"}
                </h2>
                <p>
                  {userCode
                    ? "确认设备验证码并授权 ostool CLI。"
                    : registering
                      ? "创建账号，开始管理你的设备。"
                      : "登录后进入个人工作台，查看租赁和申请设备。"}
                </p>
              </div>
              {userCode && (
                <>
                  <div className="auth-code">
                    <span>设备验证码</span>
                    <code>{userCode}</code>
                  </div>
                  <p className="auth-device-warning">
                    仅批准您刚在自己的终端发起的登录，请核对验证码，不要代他人批准登录链接。
                  </p>
                </>
              )}
              {registering && registrationAllowed === false && (
                <p role="alert">当前站点未开放注册，请联系管理员。</p>
              )}
              {!challenge ? (
                <>
                  {registering && (
                    <div className="auth-identity-fields">
                      <TextField
                        label="用户名"
                        required
                        minLength={3}
                        maxLength={40}
                        pattern="[A-Za-z0-9][A-Za-z0-9_-]{2,39}"
                        autoComplete="username"
                        placeholder="3–40 位字母、数字、_ 或 -"
                        value={username}
                        onValue={setUsername}
                      />
                      <TextField
                        label="显示名称"
                        value={displayName}
                        onValue={setDisplayName}
                        placeholder="你的名字"
                        hint=""
                      />
                    </div>
                  )}
                  <TextField
                    label={registering ? "邮箱" : "账号"}
                    type={registering ? "email" : "text"}
                    required
                    value={email}
                    onValue={(value) => {
                      setEmail(value);
                      setEmailChallenge(undefined);
                      setEmailCode("");
                    }}
                    autoComplete={registering ? "email" : "username"}
                    placeholder={
                      registering ? "you@company.com" : "输入用户名或邮箱"
                    }
                    hint=""
                  />

                  <TextField
                    label="密码"
                    type="password"
                    required
                    maxLength={1024}
                    minLength={registering ? 15 : undefined}
                    value={password}
                    onValue={setPassword}
                    autoComplete={
                      registering ? "new-password" : "current-password"
                    }
                    placeholder={
                      registering ? "至少 15 个字符" : "输入登录密码"
                    }
                  />
                  {registering && (
                    <TextField
                      label="确认密码"
                      type="password"
                      required
                      minLength={15}
                      maxLength={1024}
                      value={confirmPassword}
                      onValue={setConfirmPassword}
                      autoComplete="new-password"
                      placeholder="再次输入密码"
                    />
                  )}
                  {emailRequired && (
                    <div className="security-form">
                      {emailChallenge ? (
                        <>
                          <TextField
                            label="邮箱验证码"
                            value={emailCode}
                            onValue={setEmailCode}
                            required
                            maxLength={6}
                            inputMode="numeric"
                          />
                          <p role="status">
                            验证码已发送，签发后 {emailChallenge.expires_in}{" "}
                            秒内有效。
                          </p>
                          <Button
                            type="button"
                            variant="outline"
                            disabled={resendRemaining > 0}
                            onClick={() => {
                              setEmailChallenge(undefined);
                              setEmailCode("");
                              setRefreshKey((v) => v + 1);
                            }}
                          >
                            {resendRemaining
                              ? `${resendRemaining} 秒后可重新获取验证码`
                              : "重新获取邮箱验证码"}
                          </Button>
                        </>
                      ) : (
                        <Button
                          type="button"
                          variant="outline"
                          disabled={busy || !email || !proof?.captcha_code}
                          onClick={() => {
                            setBusy(true);
                            setMessage("");
                            void api
                              .emailCode(email, "register", proof ?? {})
                              .then((value) => {
                                setEmailChallenge(value);
                                setEmailUntil(
                                  Date.now() + value.resend_after * 1000,
                                );
                              })
                              .catch((e) => setMessage(e.message))
                              .finally(() => {
                                setBusy(false);
                                setRefreshKey((v) => v + 1);
                              });
                          }}
                        >
                          发送邮箱验证码
                        </Button>
                      )}
                    </div>
                  )}
                  <HumanVerification
                    action={
                      emailRequired && !emailChallenge
                        ? "registration_email"
                        : registering
                          ? "register"
                          : "login"
                    }
                    refreshKey={refreshKey}
                    disabled={busy}
                    onChange={setProof}
                  />
                </>
              ) : (
                <TextField
                  label="验证码或恢复码"
                  value={otp}
                  onValue={setOtp}
                  autoComplete="one-time-code"
                  maxLength={128}
                  required
                />
              )}
              {query.get("password_changed") === "1" && (
                <p role="status">密码已更新，请使用新密码重新登录。</p>
              )}
              {message && (
                <p className="auth-message" role="alert">
                  {message}
                </p>
              )}
              {!challenge && (
                <AuthLegal
                  registration={registering}
                  accepted={accepted}
                  onChange={setAccepted}
                />
              )}
              {userCode && (
                <label className="auth-device-confirmation">
                  <Checkbox
                    checked={deviceConfirmed}
                    onCheckedChange={(value) =>
                      setDeviceConfirmed(value === true)
                    }
                    aria-label="确认由我发起并核对终端验证码"
                  />
                  <span>确认由我发起并核对终端验证码</span>
                </label>
              )}
              <Button
                className="auth-submit"
                type="submit"
                disabled={
                  busy ||
                  (!!userCode && !deviceConfirmed) ||
                  (!challenge && !proof?.captcha_code) ||
                  (registering &&
                    (registrationAllowed !== true || !accepted)) ||
                  (emailRequired && (!emailChallenge || !emailCode))
                }
              >
                {busy
                  ? "处理中…"
                  : challenge
                    ? "验证并登录"
                    : userCode
                      ? "登录并批准"
                      : registering
                        ? "创建账号"
                        : "登录"}{" "}
                <ArrowRight size={15} />
              </Button>
              {challenge && (
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => {
                    setChallenge(undefined);
                    setOtp("");
                    setRefreshKey((v) => v + 1);
                  }}
                >
                  重新登录
                </Button>
              )}
              {!registering &&
                (authConfig.password_reset_available ??
                  authConfig.email_security_available) && (
                  <Link to="/password-reset">忘记密码</Link>
                )}
              {!userCode && (
                <Link
                  className="auth-switch"
                  to={registering ? "/login" : "/register"}
                >
                  {registering ? "已有账号？返回登录" : "还没有账号？免费注册"}
                </Link>
              )}
            </form>
          </div>
        </section>
      </main>
    </div>
  );
}

export default function PublicApp() {
  return (
    <ThemeProvider
      attribute="class"
      defaultTheme="system"
      enableSystem
      storageKey="ostool.web.theme"
      nonce={
        document.querySelector<HTMLMetaElement>('meta[name="csp-nonce"]')
          ?.content
      }
      disableTransitionOnChange
    >
      <EditionProvider>
        <SiteSettingsProvider>
          <SiteSessionProvider>
            <SecurityVerification />
            <div className="site-root">
              <PublicRoutes />
            </div>
            <Toaster position="top-right" richColors closeButton />
          </SiteSessionProvider>
        </SiteSettingsProvider>
      </EditionProvider>
    </ThemeProvider>
  );
}
function LegalPage() {
  const view = useSiteSettings(),
    privacy = useLocation().pathname.endsWith("/privacy"),
    s = view.settings;
  useEffect(() => {
    document.title =
      (privacy ? s.privacy_title : s.terms_title) + " · " + s.name;
  }, [privacy, s]);
  const published = privacy
    ? view.privacy_published_at
    : view.terms_published_at;
  const contact = privacy
    ? s.privacy_contact_email || s.support_email
    : s.support_email;
  return (
    <PublicLayout>
      <main className="legal-reader">
        <h1>{privacy ? s.privacy_title : s.terms_title}</h1>
        <p className="muted">
          版本 {privacy ? view.privacy_version : view.terms_version}
          {published ? " · " + new Date(published).toLocaleDateString() : ""}
        </p>
        <article>{privacy ? s.privacy_body : s.terms_body}</article>
        {contact && (
          <p>
            联系邮箱：
            <a href={"mailto:" + contact}>{contact}</a>
          </p>
        )}
      </main>
    </PublicLayout>
  );
}
function PublicRoutes() {
  const publicEdition = useEdition().edition === "public";
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Home />} />
        <Route path="/devices" element={<Devices />} />
        <Route path="/docs" element={<Docs />} />
        <Route path="/legal/terms" element={<LegalPage />} />
        <Route path="/legal/privacy" element={<LegalPage />} />
        <Route path="/about" element={<About />} />
        <Route
          path="/login"
          element={publicEdition ? <Login /> : <Navigate replace to="/" />}
        />
        <Route
          path="/password-reset"
          element={
            publicEdition ? (
              <PublicLayout>
                <main className="auth-shell">
                  <div className="auth-aside">
                    <h1>安全恢复账号</h1>
                    <p>验证邮箱后设置新密码。</p>
                  </div>
                  <PasswordReset />
                </main>
              </PublicLayout>
            ) : (
              <Navigate replace to="/" />
            )
          }
        />
        <Route
          path="/register"
          element={publicEdition ? <Login /> : <Navigate replace to="/" />}
        />
        <Route
          path="/account/*"
          element={
            publicEdition ? (
              <PublicLayout>
                <MemberWorkspace />
              </PublicLayout>
            ) : (
              <Navigate replace to="/" />
            )
          }
        />
        <Route path="*" element={<Home />} />
      </Routes>
    </BrowserRouter>
  );
}

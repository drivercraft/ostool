import { FormEvent, useState } from "react";
import {
  ArrowRight, CheckCircle2, CircuitBoard, Command, FileText, Github,
  Menu, Radio, ShieldCheck, Terminal, X, Zap,
} from "lucide-react";
import {
  BrowserRouter, Link, NavLink, Route, Routes, useLocation, useNavigate,
} from "react-router-dom";
import { api, saveAuthToken } from "./api/client";
import { Button } from "./components/ui/button";

const publicNavigation = [
  ["/", "首页"], ["/devices", "设备"], ["/docs", "文档"], ["/about", "关于 ostool"],
] as const;

function Logo({ light = false }: { light?: boolean }) {
  return <Link className={`site-logo${light ? " site-logo-light" : ""}`} to="/"><span className="logo-mark"><CircuitBoard size={18} strokeWidth={2.5} /></span><span>ostool</span></Link>;
}

function SiteHeader() {
  const [open, setOpen] = useState(false);
  return <header className="site-header"><div className="site-header-inner"><Logo /><nav className={open ? "site-nav site-nav-open" : "site-nav"} aria-label="网站导航">{publicNavigation.map(([href, label]) => <NavLink key={href} to={href} end={href === "/"} onClick={() => setOpen(false)}>{label}</NavLink>)}<div className="mobile-nav-actions"><Link to="/login" onClick={() => setOpen(false)}>登录</Link><Button asChild size="sm"><Link to="/login?register=1" onClick={() => setOpen(false)}>免费注册</Link></Button></div></nav><div className="site-actions"><Link className="site-login" to="/login">登录</Link><Button asChild size="sm"><Link to="/login?register=1">免费注册</Link></Button></div><button className="site-menu-toggle" aria-label={open ? "关闭菜单" : "打开菜单"} onClick={() => setOpen((value) => !value)}>{open ? <X size={20} /> : <Menu size={20} />}</button></div></header>;
}

function SiteFooter() {
  return <footer className="site-footer"><div className="site-footer-main"><div className="footer-brand"><Logo light /><p>为嵌入式团队打造的开发板协作平台。</p></div><div className="footer-column"><strong>产品</strong><Link to="/devices">设备管理</Link><Link to="/docs">文档中心</Link><Link to="/login">管理后台</Link></div><div className="footer-column"><strong>资源</strong><Link to="/docs#cli">ostool CLI</Link><Link to="/docs#api">API 参考</Link><a href="https://github.com/drivercraft/ostool">GitHub</a></div><div className="footer-column"><strong>开始使用</strong><span>连接你的第一块开发板</span><Button asChild variant="outline" size="sm"><Link to="/login">进入控制台 <ArrowRight size={14} /></Link></Button></div></div><div className="site-footer-bottom"><span>© {new Date().getFullYear()} ostool. Built for hardware teams.</span><span className="footer-social"><Github size={15} /> Open source tooling</span></div></footer>;
}

function PublicLayout({ children }: { children: React.ReactNode }) {
  return <div className="public-site"><SiteHeader />{children}<SiteFooter /></div>;
}

function Home() {
  return <PublicLayout><main className="public-home"><section className="public-hero-section"><div className="hero-glow hero-glow-one" /><div className="hero-glow hero-glow-two" /><div className="hero-copy"><div className="hero-kicker"><span className="status-pulse" /> 开发板协作平台 <span className="hero-kicker-divider" /> v0.9</div><h1>让每一块开发板，<em>随时可用。</em></h1><p className="hero-subtitle">从设备租约、串口会话到 OTA 发布，ostool 将分散的硬件工作流组织成一个清晰、可靠的团队平台。</p><div className="hero-actions"><Button asChild size="lg"><Link to="/login?register=1">开始使用 <ArrowRight size={16} /></Link></Button><Button asChild size="lg" variant="outline"><Link to="/docs">查看文档 <FileText size={16} /></Link></Button></div><div className="hero-trust"><span><CheckCircle2 size={15} /> 开源工具链</span><span><ShieldCheck size={15} /> 私有化部署</span><span><Zap size={15} /> 实时状态同步</span></div></div><div className="hero-console" aria-label="ostool 控制台预览"><div className="console-window-bar"><span className="window-dot red" /><span className="window-dot yellow" /><span className="window-dot green" /><span className="console-address">app.ostool.dev / overview</span></div><div className="console-body"><aside><div className="console-mini-logo"><span className="logo-mark"><CircuitBoard size={12} /></span> ostool</div><span className="console-nav active"><Radio size={13} /> 总览</span><span className="console-nav"><CircuitBoard size={13} /> 开发板</span><span className="console-nav"><Terminal size={13} /> 会话</span><span className="console-nav"><FileText size={13} /> 文件</span></aside><div className="console-content"><div className="console-heading"><div><span>Workspace / Overview</span><strong>设备总览</strong></div><span className="console-live"><i /> LIVE</span></div><div className="console-metrics"><div><span>在线设备</span><strong>12</strong><small>+3 本周</small></div><div><span>活跃会话</span><strong>08</strong><small>稳定运行</small></div><div><span>可用率</span><strong>99.8%</strong><small>过去 30 天</small></div></div><div className="console-chart"><div className="chart-label"><span>设备连接趋势</span><small>最近 24 小时</small></div><div className="chart-bars">{[35,48,42,67,54,76,62,91,74,84,68,96].map((height, index) => <i key={index} style={{ height: `${height}%` }} />)}</div></div></div></div></div></section><section className="logo-strip"><span>为硬件团队提供稳定的日常基础设施</span><div><b>UART</b><b>QEMU</b><b>U-BOOT</b><b>OTA</b><b>PXE</b></div></section><section className="public-section feature-section"><div className="section-heading"><span className="section-label">ONE WORKSPACE</span><h2>硬件开发，也可以很从容。</h2><p>把设备、文件、会话和权限放在同一个上下文里，让团队更快地发现问题、复现问题和交付结果。</p></div><div className="feature-grid"><FeatureCard icon={<CircuitBoard />} number="01" title="设备资产" text="统一管理实体开发板与 QEMU 虚拟设备，实时了解在线状态、固件和当前占用者。" /><FeatureCard icon={<Radio />} number="02" title="实时会话" text="串口、TFTP、HTTP Boot 和电源控制协同工作，租约到期后自动释放资源。" /><FeatureCard icon={<Command />} number="03" title="CLI 优先" text="浏览器和 ostool CLI 使用同一套登录与 API 契约，脚本和人工操作自然衔接。" /></div></section><section className="public-section split-section"><div className="split-visual"><div className="terminal-card"><div className="terminal-top"><span><i /><i /><i /></span><small>ostool terminal</small></div><div className="terminal-lines"><p><b>$</b> ostool login</p><p className="dim">Opening device authorization page...</p><p><b>✓</b> Authenticated as <strong>team@acme.dev</strong></p><p><b>$</b> ostool board run --type rk3568</p><p className="blue">● lease acquired · serial connected</p></div></div></div><div className="split-copy"><span className="section-label">MADE FOR BUILDERS</span><h2>从终端到浏览器，保持同一套体验。</h2><p>开发、测试和运维不需要在多个工具之间来回切换。ostool 用清晰的 API、可追踪的事件和安全的会话，把硬件访问变成团队基础设施。</p><ul><li><CheckCircle2 />设备租约避免资源冲突</li><li><CheckCircle2 />SSE 实时推送，无需手动刷新</li><li><CheckCircle2 />OAuth 登录与个人访问令牌</li></ul><Link className="text-arrow-link" to="/docs">了解工作方式 <ArrowRight size={15} /></Link></div></section><section className="public-cta"><div><span className="section-label">READY WHEN YOU ARE</span><h2>从第一块板卡开始，建立你的硬件工作台。</h2><p>几分钟完成部署，随后专注于真正重要的开发工作。</p></div><Button asChild size="lg"><Link to="/login?register=1">创建免费账号 <ArrowRight size={16} /></Link></Button></section></main></PublicLayout>;
}

function FeatureCard({ icon, number, title, text }: { icon: React.ReactNode; number: string; title: string; text: string }) {
  return <article className="feature-card"><div className="feature-card-top"><span className="feature-icon">{icon}</span><span className="feature-number">{number}</span></div><h3>{title}</h3><p>{text}</p><ArrowRight className="feature-arrow" size={17} /></article>;
}

function Devices() {
  return <PublicLayout><main className="public-inner-page"><div className="inner-hero"><span className="section-label">DEVICE CLOUD</span><h1>设备，在这里保持可见。</h1><p>连接真实开发板或 QEMU 虚拟设备，团队成员可以在同一个工作区中发现、分配和操作它们。</p></div><div className="device-showcase"><div className="device-showcase-card"><CircuitBoard size={28} /><strong>设备租约</strong><span>按需分配，自动释放</span></div><div className="device-showcase-card"><Radio size={28} /><strong>串口会话</strong><span>实时传输，状态透明</span></div><div className="device-showcase-card"><Zap size={28} /><strong>远程电源</strong><span>支持多种电源控制器</span></div></div><div className="inner-callout"><div><h2>准备好连接设备了吗？</h2><p>登录管理后台，查看你的设备池和当前会话。</p></div><Button asChild><Link to="/login">进入控制台 <ArrowRight size={15} /></Link></Button></div></main></PublicLayout>;
}

function Docs() {
  return <PublicLayout><main className="public-inner-page docs-page"><div className="inner-hero"><span className="section-label">DOCUMENTATION</span><h1>清晰的文档，少走弯路。</h1><p>从第一次安装到自动化流水线，按你的工作方式查找 ostool 的使用指南。</p></div><div className="docs-grid"><a id="cli" className="docs-card" href="https://github.com/drivercraft/ostool"><Terminal /><div><strong>ostool CLI</strong><span>登录、构建、启动和串口会话的命令行工作流。</span></div><ArrowRight /></a><a id="api" className="docs-card" href="https://github.com/drivercraft/ostool"><FileText /><div><strong>API 参考</strong><span>服务端 API、实时事件和设备会话契约。</span></div><ArrowRight /></a><a className="docs-card" href="https://github.com/drivercraft/ostool"><Command /><div><strong>部署指南</strong><span>PostgreSQL、systemd 和私有化部署配置。</span></div><ArrowRight /></a></div></main></PublicLayout>;
}

function About() {
  return <PublicLayout><main className="public-inner-page about-page"><div className="inner-hero"><span className="section-label">OPEN SOURCE</span><h1>为真实硬件而生的工具。</h1><p>ostool 是一个面向嵌入式开发团队的开源工具链，连接构建、启动和设备管理的每一个环节。</p><Button asChild><a href="https://github.com/drivercraft/ostool">在 GitHub 上查看 <Github size={15} /></a></Button></div></main></PublicLayout>;
}

function Login() {
  const navigate = useNavigate();
  const location = useLocation();
  const query = new URLSearchParams(location.search);
  const userCode = query.get("user_code") ?? "";
  const [registering, setRegistering] = useState(query.get("register") === "1");
  const [email, setEmail] = useState(""); const [password, setPassword] = useState(""); const [displayName, setDisplayName] = useState(""); const [message, setMessage] = useState(""); const [busy, setBusy] = useState(false);
  async function submit(event: FormEvent) {
    event.preventDefault(); setBusy(true); setMessage("");
    try { const token = registering ? await api.register(email, password, displayName) : await api.login(email, password); saveAuthToken(token); if (userCode) { await api.authorizeDevice(userCode); setMessage("设备登录已批准，可以返回 ostool CLI。"); } else navigate("/admin/overview"); }
    catch (error) { setMessage(error instanceof Error ? error.message : "登录失败"); } finally { setBusy(false); }
  }
  return <div className="auth-page"><SiteHeader /><main className="auth-shell"><div className="auth-aside"><span className="section-label">WELCOME TO OSTOOL</span><h1>把硬件工作，<em>组织起来。</em></h1><p>登录你的工作区，管理设备、会话和团队访问权限。</p><div className="auth-aside-points"><span><CheckCircle2 /> 统一的设备工作区</span><span><ShieldCheck /> 安全的团队访问</span></div></div><form className="auth-card" onSubmit={submit}><Logo /><div><h2>{userCode ? "批准 CLI 登录" : registering ? "创建你的账号" : "欢迎回来"}</h2><p>{userCode ? "确认设备验证码并授权 ostool CLI。" : registering ? "创建账号，开始管理你的设备。" : "登录后进入设备管理控制台。"}</p></div>{userCode && <div className="auth-code"><span>设备验证码</span><code>{userCode}</code></div>}<label>邮箱<input type="email" required value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="email" placeholder="you@company.com" /></label>{registering && <label>显示名称<input value={displayName} onChange={(e) => setDisplayName(e.target.value)} placeholder="你的名字" /></label>}<label>密码<input type="password" required minLength={12} value={password} onChange={(e) => setPassword(e.target.value)} autoComplete={registering ? "new-password" : "current-password"} placeholder="至少 12 个字符" /></label>{message && <p className="auth-message" role="alert">{message}</p>}<Button className="auth-submit" type="submit" disabled={busy}>{busy ? "处理中…" : userCode ? "登录并批准" : registering ? "创建账号" : "登录控制台"} <ArrowRight size={15} /></Button>{!userCode && <button type="button" className="auth-switch" onClick={() => setRegistering((value) => !value)}>{registering ? "已有账号？返回登录" : "还没有账号？免费注册"}</button>}<small className="auth-legal">继续即表示你同意 ostool 的服务条款与隐私政策。</small></form></main><SiteFooter /></div>;
}

export default function PublicApp() {
  return <BrowserRouter><Routes><Route path="/" element={<Home />} /><Route path="/devices" element={<Devices />} /><Route path="/docs" element={<Docs />} /><Route path="/about" element={<About />} /><Route path="/login" element={<Login />} /><Route path="*" element={<Home />} /></Routes></BrowserRouter>;
}

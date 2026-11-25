import { toast } from "sonner";
import { MessageToast } from "@/components/message-toast";
import {
  SiteSettingsProvider,
  useSiteSettings,
} from "@/components/site-settings";
import { safeWebsiteRedirect } from "@/auth-session";
import { NotFound } from "@/components/not-found";
import { AuthLegal } from "@/components/auth-legal";
import { Checkbox } from "@/components/ui/checkbox";
import { FormEvent, useEffect, useState } from "react";
import {
  ArrowRight,
  CheckCircle2,
  CircuitBoard,
  Github,
  ShieldCheck,
  Terminal,
} from "lucide-react";
import {
  BrowserRouter,
  Link,
  Navigate,
  Route,
  Routes,
  useLocation,
  useNavigate,
} from "react-router-dom";
import { EditionProvider, useEdition } from "@/edition";
import MemberWorkspace from "@/apps/web/pages/MemberWorkspace";
import DeviceDetail from "@/apps/web/pages/DeviceDetail";
import RentalRequest from "@/apps/web/pages/RentalRequest";
import Home from "@/apps/web/pages/Home";
import {
  Logo,
  PublicLayout,
  SiteScrollRestoration,
} from "@/apps/web/components/site-layout";
import { BoardVisual } from "@/apps/web/components/board-visual";
import { Documentation } from "@/apps/web/pages/Documentation";
import { DeviceCatalog } from "@/apps/web/pages/DeviceCatalog";
import { SiteSessionProvider } from "@/apps/web/session";
import { ThemeProvider } from "next-themes";
import { Toaster } from "@/components/ui/sonner";
import { RegistrationForm } from "@/apps/web/components/registration-form";
import { TextField } from "@/components/forms";
import { HumanVerification } from "@/components/human-verification";
import { PasswordReset } from "@/components/password-reset";
import { SecurityVerification } from "@/components/security-verification";
import { api, notifyAuthChanged, type HumanProof } from "@/api/client";
import { Button } from "@/components/ui/button";

function Devices() {
  const publicEdition = useEdition().enabled;
  return (
    <PublicLayout>
      <main className="public-inner-page devices-page">
        <section className="device-product-hero">
          <div className="inner-hero">
            <span className="section-label">YOUR HARDWARE. ONE WORKSPACE.</span>
            <h1>
              让硬件资源，<em>成为研发的助力。</em>
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
            <a href={publicEdition ? "/account" : "/qcl/overview"}>
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
  const [approvedCode, setApprovedCode] = useState("");
  const deviceApproved = !!userCode && approvedCode === userCode;
  const [deviceConfirmed, setDeviceConfirmed] = useState(false);
  useEffect(() => setDeviceConfirmed(false), [userCode]);
  const registering = location.pathname === "/register";
  const authConfig = useEdition();
  const legalSite = useSiteSettings();
  useEffect(() => {
    document.title =
      (deviceApproved
        ? "登录成功"
        : userCode
          ? "批准 CLI 登录"
          : registering
            ? "注册"
            : "登录") +
      " · " +
      legalSite.settings.name;
  }, [deviceApproved, registering, userCode, legalSite.settings.name]);
  const [proof, setProof] = useState<HumanProof>();
  const [refreshKey, setRefreshKey] = useState(0);
  const [challenge, setChallenge] = useState<string>();
  const [otp, setOtp] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (userCode && !deviceConfirmed) {
      toast.error("请确认由您发起登录，并核对终端验证码。");
      return;
    }
    setBusy(true);
    try {
      const result = challenge
        ? await api.verifyMfa(challenge, otp)
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
        setApprovedCode(userCode);
        setPassword("");
        setOtp("");
      } else {
        navigate(safeWebsiteRedirect(query.get("redirect")));
      }
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "登录失败");
      if (!challenge) setRefreshKey((v) => v + 1);
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="auth-page auth-page-split site-auth-page">
      <main className="auth-shell">
        <aside className="auth-aside" aria-label="平台介绍">
          <Logo light />
          <div className="auth-aside-copy">
            <span className="section-label">YOUR HARDWARE WORKSPACE</span>
            <h1>
              连接每一块开发板，<em>专注下一次创造。</em>
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
          aria-label={
            deviceApproved ? "登录成功" : registering ? "注册账号" : "用户登录"
          }
        >
          <div className="auth-form-scroll">
            {deviceApproved ? (
              <div
                className="auth-card auth-success-card"
                role="status"
                aria-labelledby="cli-login-success"
                tabIndex={-1}
                ref={(node) => node?.focus()}
              >
                <CheckCircle2
                  className="auth-success-icon"
                  aria-hidden="true"
                />
                <div>
                  <h2 id="cli-login-success">登录成功</h2>
                  <p>设备登录已批准，可以返回 ostool CLI。</p>
                  <p>你可以关闭此页面，回到终端继续操作。</p>
                </div>
              </div>
            ) : registering ? (
              <RegistrationForm />
            ) : (
              <form className="auth-card" onSubmit={submit}>
                <div>
                  <h2>{userCode ? "批准 CLI 登录" : "欢迎回来"}</h2>
                  <p>
                    {userCode
                      ? "确认设备验证码并授权 ostool CLI。"
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
                {!challenge ? (
                  <>
                    <TextField
                      label="账号"
                      required
                      value={email}
                      onValue={setEmail}
                      autoComplete="username"
                      placeholder="输入用户名或邮箱"
                    />
                    <TextField
                      label="密码"
                      type="password"
                      required
                      maxLength={1024}
                      value={password}
                      onValue={setPassword}
                      autoComplete="current-password"
                      placeholder="输入登录密码"
                    />
                    <HumanVerification
                      action="login"
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
                  <MessageToast
                    kind="success"
                    message="密码已更新，请使用新密码重新登录。"
                  />
                )}

                {!challenge && <AuthLegal />}
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
                    (!challenge && !proof?.captcha_code)
                  }
                >
                  {busy
                    ? "处理中…"
                    : challenge
                      ? "验证并登录"
                      : userCode
                        ? "登录并批准"
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
                {authConfig.password_reset_available && (
                  <Link to="/password-reset">忘记密码</Link>
                )}
                {!userCode && (
                  <Link className="auth-switch" to="/register">
                    还没有账号？免费注册
                  </Link>
                )}
              </form>
            )}
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
          </SiteSessionProvider>
        </SiteSettingsProvider>
      </EditionProvider>
      <Toaster />
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
  const publicEdition = useEdition().enabled;
  return (
    <BrowserRouter>
      <SiteScrollRestoration />
      <Routes>
        <Route path="/" element={<Home />} />
        <Route path="/devices" element={<Devices />} />
        <Route
          path="/devices/:boardId"
          element={
            <PublicLayout>
              <DeviceDetail />
            </PublicLayout>
          }
        />
        <Route path="/docs" element={<Docs />} />
        <Route
          path="/legal/terms"
          element={publicEdition ? <LegalPage /> : <Navigate replace to="/" />}
        />
        <Route
          path="/legal/privacy"
          element={publicEdition ? <LegalPage /> : <Navigate replace to="/" />}
        />
        <Route path="/about" element={<About />} />
        <Route
          path="/login"
          element={
            publicEdition ? (
              <PublicLayout>
                <Login />
              </PublicLayout>
            ) : (
              <Navigate replace to="/" />
            )
          }
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
          element={
            publicEdition ? (
              <PublicLayout>
                <Login />
              </PublicLayout>
            ) : (
              <Navigate replace to="/" />
            )
          }
        />
        <Route
          path="/rental-request"
          element={
            publicEdition ? (
              <PublicLayout>
                <RentalRequest />
              </PublicLayout>
            ) : (
              <Navigate replace to="/" />
            )
          }
        />
        <Route
          path="/account/requests/new"
          element={<LegacyRentalRequestRedirect />}
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
        <Route
          path="*"
          element={
            <PublicLayout>
              <main className="public-inner-page">
                <NotFound />
              </main>
            </PublicLayout>
          }
        />
      </Routes>
    </BrowserRouter>
  );
}

function LegacyRentalRequestRedirect() {
  const location = useLocation();
  return <Navigate replace to={`/rental-request${location.search}`} />;
}

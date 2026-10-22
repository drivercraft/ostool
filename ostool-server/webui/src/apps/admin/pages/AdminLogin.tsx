import { AuthLegal } from "@/components/auth-legal";
import { useSiteSettings } from "@/components/site-settings";
import { useEffect, useState, type FormEvent } from "react";
import { CircuitBoard, ArrowRight, ShieldCheck } from "lucide-react";
import { api, notifyAuthChanged, type HumanProof } from "@/api/client";
import { HumanVerification } from "@/components/human-verification";
import { useEdition } from "@/edition";
import { safeAdminRedirect } from "@/auth-session";
import { ThemeMenu } from "@/components/theme-menu";
import { Button } from "@/components/ui/button";
import { TextField, Notice } from "@/components/forms";

export default function AdminLogin() {
  const [email, setEmail] = useState(""),
    [password, setPassword] = useState("");
  const config = useEdition();
  const { settings } = useSiteSettings();
  const [proof, setProof] = useState<HumanProof>();
  const [refreshKey, setRefreshKey] = useState(0);
  const [challenge, setChallenge] = useState<string>();
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  useEffect(() => {
    document.title = "后台登录 · ostool";
  }, []);
  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError("");
    try {
      const result = challenge
        ? await api.verifyMfa(challenge, code)
        : await api.adminLogin(email, password, proof ?? {});
      if (result.mfa_required) {
        setChallenge(result.challenge_id);
        setPassword("");
        setCode("");
        return;
      }
      notifyAuthChanged();
      window.location.replace(
        safeAdminRedirect(
          new URLSearchParams(window.location.search).get("redirect"),
        ),
      );
    } catch (error) {
      setError(error instanceof Error ? error.message : "后台登录失败");
      if (!challenge) setRefreshKey((value) => value + 1);
    } finally {
      setBusy(false);
    }
  }
  return (
    <main className="admin-login-page">
      <header className="admin-login-header">
        <a href="/admin/login" className="admin-login-brand">
          <span className="logo-mark">
            {settings.logo_url ? (
              <img
                src={settings.logo_url}
                alt=""
                className="size-full object-contain"
              />
            ) : (
              <CircuitBoard size={19} />
            )}
          </span>
          <strong>{settings.short_name}</strong>
          <span>管理后台</span>
        </a>
        <ThemeMenu label="切换后台主题" />
      </header>
      <form className="admin-login-card" onSubmit={submit}>
        <div className="admin-login-heading">
          <span className="admin-login-icon" aria-hidden="true">
            <ShieldCheck size={25} />
          </span>
          <h1>管理后台登录</h1>
        </div>
        <p>使用管理员账号登录开发板管理后台。</p>
        {!challenge ? (
          <>
            <TextField
              label="用户名或邮箱"
              type="text"
              autoComplete="username"
              placeholder="输入用户名或邮箱"
              hint=""
              required
              value={email}
              onValue={setEmail}
            />
            <TextField
              label="密码"
              type="password"
              autoComplete="current-password"
              placeholder="输入登录密码"
              required
              maxLength={1024}
              value={password}
              onValue={setPassword}
            />
            <HumanVerification
              action="admin_login"
              refreshKey={refreshKey}
              disabled={busy}
              onChange={setProof}
            />
          </>
        ) : (
          <TextField
            label="验证码或恢复码"
            placeholder="输入验证码或恢复码"
            value={code}
            onValue={setCode}
            autoComplete="one-time-code"
            maxLength={128}
            required
          />
        )}
        {error && <Notice>{error}</Notice>}
        {!challenge && <AuthLegal />}
        <Button
          type="submit"
          disabled={busy || (!challenge && !proof?.captcha_code)}
        >
          {busy ? "正在登录…" : challenge ? "验证并登录" : "登录"}
          <ArrowRight size={17} />
        </Button>
        {challenge && (
          <Button
            type="button"
            variant="outline"
            onClick={() => {
              setChallenge(undefined);
              setRefreshKey((v) => v + 1);
              setCode("");
            }}
          >
            重新登录
          </Button>
        )}
        {(config.password_reset_available ??
          config.email_security_available) && (
          <a href="/password-reset?surface=admin">忘记密码</a>
        )}
        <a className="admin-login-site" href="/">
          返回网站首页
        </a>
      </form>
      <footer>ostool · 开发板管理后台</footer>
    </main>
  );
}

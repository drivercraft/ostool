import { useEffect, useState, type FormEvent } from "react";
import { CircuitBoard, ArrowRight, ShieldCheck } from "lucide-react";
import { api, saveAuthToken } from "@/api/client";
import { safeAdminRedirect } from "@/auth-session";
import { SiteThemeMenu } from "@/components/site-theme-menu";
import { Button } from "@/components/ui/button";
import { TextField, Notice } from "@/components/forms";

export default function AdminLogin() {
  const [email, setEmail] = useState(""),
    [password, setPassword] = useState("");
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
      const grant = await api.adminLogin(email, password);
      saveAuthToken(grant);
      window.location.replace(
        safeAdminRedirect(
          new URLSearchParams(window.location.search).get("redirect"),
        ),
      );
    } catch (error) {
      setError(error instanceof Error ? error.message : "后台登录失败");
    } finally {
      setBusy(false);
    }
  }
  return (
    <main className="admin-login-page">
      <header className="admin-login-header">
        <a href="/admin/login" className="admin-login-brand">
          <span className="logo-mark">
            <CircuitBoard size={19} />
          </span>
          <strong>ostool</strong>
          <span>管理后台</span>
        </a>
        <SiteThemeMenu label="切换后台主题" />
      </header>
      <form className="admin-login-card" onSubmit={submit}>
        <span className="admin-login-icon">
          <ShieldCheck size={25} />
        </span>
        <h1>管理后台登录</h1>
        <p>使用管理员账号登录开发板管理后台。</p>
        <TextField
          label="邮箱"
          type="email"
          autoComplete="username"
          required
          value={email}
          onValue={setEmail}
        />
        <TextField
          label="密码"
          type="password"
          autoComplete="current-password"
          required
          maxLength={1024}
          value={password}
          onValue={setPassword}
        />
        {error && <Notice>{error}</Notice>}
        <Button type="submit" disabled={busy}>
          {busy ? "正在登录…" : "登录"}
          <ArrowRight size={17} />
        </Button>
        <a className="admin-login-site" href="/">
          返回网站首页
        </a>
      </form>
      <footer>ostool · 开发板管理后台</footer>
    </main>
  );
}

import { FormEvent, useState } from "react";
import { BrowserRouter, Link, Route, Routes, useLocation, useNavigate } from "react-router-dom";
import { api, saveAuthToken } from "./api/client";
import { Button } from "./components/ui/button";

function Home() {
  return <main className="public-shell"><section className="public-hero"><p className="public-eyebrow">OSTOOL BOARD PLATFORM</p><h1>让开发板管理回到一个清晰的工作流。</h1><p className="public-lede">统一管理开发板、串口会话、启动文件和 OTA 流程。使用账号登录后即可从浏览器或 ostool CLI 访问团队资源。</p><div className="public-actions"><Button asChild size="lg"><Link to="/login">登录控制台</Link></Button><Button asChild variant="outline" size="lg"><a href="/admin/overview">打开管理后台</a></Button></div></section><section className="public-features"><article><strong>板卡租约</strong><span>避免多人同时占用同一块开发板。</span></article><article><strong>实时会话</strong><span>串口、启动文件和状态通过一个管理入口同步。</span></article><article><strong>CLI 兼容</strong><span>支持 ostool 的 Device Authorization 登录流程。</span></article></section></main>;
}

function Login() {
  const navigate = useNavigate();
  const location = useLocation();
  const userCode = new URLSearchParams(location.search).get("user_code") ?? "";
  const [registering, setRegistering] = useState(false);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  async function submit(event: FormEvent) {
    event.preventDefault(); setBusy(true); setMessage("");
    try {
      const token = registering ? await api.register(email, password, displayName) : await api.login(email, password);
      saveAuthToken(token);
      if (userCode) { await api.authorizeDevice(userCode); setMessage("设备登录已批准，可以返回 ostool CLI。"); }
      else navigate("/admin/overview");
    } catch (error) { setMessage(error instanceof Error ? error.message : "登录失败"); }
    finally { setBusy(false); }
  }
  return <main className="auth-shell"><form className="auth-card" onSubmit={submit}><Link className="auth-brand" to="/">ostool</Link><h1>{userCode ? "批准 CLI 登录" : registering ? "创建账号" : "登录 ostool"}</h1>{userCode && <p className="auth-hint">设备验证码：<code>{userCode}</code></p>}<label>邮箱<input type="email" required value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="email" /></label>{registering && <label>显示名称<input value={displayName} onChange={(e) => setDisplayName(e.target.value)} /></label>}<label>密码<input type="password" required minLength={12} value={password} onChange={(e) => setPassword(e.target.value)} autoComplete={registering ? "new-password" : "current-password"} /></label>{message && <p className="auth-message" role="alert">{message}</p>}<Button type="submit" disabled={busy}>{busy ? "处理中…" : userCode ? "登录并批准" : registering ? "创建账号" : "登录"}</Button>{!userCode && <button type="button" className="auth-switch" onClick={() => setRegistering((v) => !v)}>{registering ? "已有账号？登录" : "创建新账号"}</button>}</form></main>;
}

export default function PublicApp() {
  return <BrowserRouter><Routes><Route path="/" element={<Home />} /><Route path="/login" element={<Login />} /><Route path="*" element={<Home />} /></Routes></BrowserRouter>;
}

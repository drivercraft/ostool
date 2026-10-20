import { useEffect, useState } from "react";
import { api, clearAuthToken, type AuthUser } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Notice } from "@/components/forms";

export default function Account() {
  const [user, setUser] = useState<AuthUser>();
  const [name, setName] = useState("ostool CLI");
  const [days, setDays] = useState("90");
  const [token, setToken] = useState("");
  const [error, setError] = useState("");
  useEffect(() => { api.me().then(setUser).catch((e) => setError(e instanceof Error ? e.message : "无法读取账号")); }, []);
  async function create() {
    setError(""); setToken("");
    try { const result = await api.createPersonalAccessToken(name, Number(days) || undefined); setToken(result.token); }
    catch (e) { setError(e instanceof Error ? e.message : "创建令牌失败"); }
  }
  return <div className="page"><header className="page-header"><div><p className="eyebrow">ACCOUNT</p><h1>账号与访问令牌</h1></div><Button variant="outline" onClick={() => { clearAuthToken(); window.location.href = "/login"; }}>退出登录</Button></header>{error && <Notice>{error}</Notice>}<section className="settings-grid"><article className="card"><h2>当前账号</h2><p>{user?.display_name || "未设置名称"}</p><small>{user?.email}</small><small>{user?.is_admin ? "管理员" : "成员"}</small></article><article className="card"><h2>创建个人访问令牌</h2><p className="muted">令牌只会在创建后显示一次，保存后可用于 <code>ostool login --with-token</code>。</p><label>名称<input value={name} onChange={(e) => setName(e.target.value)} /></label><label>有效期（天）<input type="number" min="1" value={days} onChange={(e) => setDays(e.target.value)} /></label><Button onClick={create}>创建令牌</Button>{token && <div className="token-result"><strong>请立即复制：</strong><code>{token}</code></div>}</article></section></div>;
}

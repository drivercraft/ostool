import { useState, type FormEvent } from "react";
import { api, type EmailChallenge, type HumanProof } from "@/api/client";
import { useEdition } from "@/edition";
import { HumanVerification } from "./human-verification";
import { Button } from "./ui/button";
import { TextField, Notice, useAction } from "./forms";
import { useCountdown } from "./security-hooks";
export function PasswordReset() {
  const available = useEdition().email_security_available;
  const [email, setEmail] = useState(""),
    [proof, setProof] = useState<HumanProof>(),
    [challenge, setChallenge] = useState<EmailChallenge>(),
    [code, setCode] = useState(""),
    [password, setPassword] = useState(""),
    [confirmation, setConfirmation] = useState(""),
    [done, setDone] = useState(false),
    [refreshKey, setRefreshKey] = useState(0),
    [until, setUntil] = useState(0);
  const task = useAction(),
    remaining = useCountdown(until);
  const login =
    new URLSearchParams(location.search).get("surface") === "admin"
      ? "/admin/login"
      : "/login";
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!challenge) {
      const ok = await task.run(async () => {
        const result = await api.emailCode(
          email,
          "password_reset",
          proof ?? {},
        );
        setChallenge(result);
        setUntil(Date.now() + result.resend_after * 1000);
      });
      if (!ok) setRefreshKey((value) => value + 1);
    } else {
      await task.run(async () => {
        if (password !== confirmation) throw new Error("两次输入的密码不一致");
        if (Array.from(password).length < 15)
          throw new Error("密码至少需要 15 个字符");
        await api.resetPassword(challenge.challenge_id, code, password);
        setPassword("");
        setConfirmation("");
        setCode("");
        setDone(true);
      });
    }
  }
  return (
    <form className="auth-card security-form" onSubmit={submit}>
      <h2>重置密码</h2>
      {!available ? (
        <Notice>本站暂未开放邮箱找回，请联系管理员。</Notice>
      ) : done ? (
        <p role="status">密码已重置，旧登录和访问令牌已失效，请重新登录。</p>
      ) : (
        <>
          <TextField
            label="邮箱"
            type="email"
            required
            maxLength={320}
            disabled={!!challenge}
            value={email}
            onValue={setEmail}
          />
          {!challenge ? (
            <HumanVerification
              action="password_reset"
              disabled={task.pending}
              refreshKey={refreshKey}
              onChange={setProof}
            />
          ) : (
            <>
              <p>
                若该邮箱对应可用账号，验证码将很快送达。验证码十分钟内有效。
              </p>
              <TextField
                label="邮箱验证码"
                required
                maxLength={6}
                inputMode="numeric"
                value={code}
                onValue={setCode}
              />
              <TextField
                label="新密码"
                type="password"
                minLength={15}
                maxLength={1024}
                required
                autoComplete="new-password"
                value={password}
                onValue={setPassword}
              />
              <TextField
                label="确认新密码"
                type="password"
                minLength={15}
                maxLength={1024}
                required
                autoComplete="new-password"
                value={confirmation}
                onValue={setConfirmation}
              />
              <Button
                type="button"
                variant="outline"
                disabled={remaining > 0 || task.pending}
                onClick={() => {
                  setChallenge(undefined);
                  setCode("");
                  setRefreshKey((value) => value + 1);
                }}
              >
                {remaining
                  ? `${remaining} 秒后可重新获取验证码`
                  : "重新获取验证码"}
              </Button>
            </>
          )}
          {task.error && <Notice>{task.error}</Notice>}
          <Button
            type="submit"
            disabled={task.pending || (!challenge && !proof?.captcha_code)}
          >
            {task.pending
              ? "处理中…"
              : challenge
                ? "重置密码"
                : "发送邮箱验证码"}
          </Button>
        </>
      )}
      <a href={login}>返回登录</a>
    </form>
  );
}

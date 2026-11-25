import { MessageToast } from "@/components/message-toast";
import { useRef, useState, type FormEvent } from "react";
import {
  api,
  ApiRequestError,
  type EmailChallenge,
  type HumanProof,
} from "@/api/client";
import { useEdition } from "@/edition";
import { HumanVerification } from "./human-verification";
import { Button } from "./ui/button";
import { TextField, Notice, useAction } from "./forms";
import { useCountdown } from "./security-hooks";
const cooldownKey = "ostool.password-reset.email-cooldown";
export function PasswordReset() {
  const config = useEdition();
  const available = config.password_reset_available;
  const [email, setEmail] = useState(""),
    [proof, setProof] = useState<HumanProof>(),
    [challenge, setChallenge] = useState<
      EmailChallenge & { expiresAt: number }
    >(),
    [code, setCode] = useState(""),
    [password, setPassword] = useState(""),
    [confirmation, setConfirmation] = useState(""),
    [done, setDone] = useState(false),
    [refreshKey, setRefreshKey] = useState(0),
    [until, setUntil] = useState(() => {
      try {
        return Number(sessionStorage.getItem(cooldownKey)) || 0;
      } catch {
        return 0;
      }
    }),
    [confirmUntil, setConfirmUntil] = useState(0);
  const inFlight = useRef(false);
  const task = useAction(),
    remaining = useCountdown(until),
    validity = useCountdown(challenge?.expiresAt ?? 0),
    confirmRemaining = useCountdown(confirmUntil);
  const expired = !!challenge && validity === 0;
  function cooldown(seconds: number) {
    const value = Date.now() + seconds * 1000;
    setUntil(value);
    try {
      sessionStorage.setItem(cooldownKey, String(value));
    } catch {
      /* Storage may be disabled. */
    }
  }
  function editEmail() {
    setChallenge(undefined);
    setCode("");
    setProof(undefined);
    setPassword("");
    setConfirmation("");
    setRefreshKey((value) => value + 1);
  }
  const login =
    new URLSearchParams(location.search).get("surface") === "admin"
      ? "/qcl/login"
      : "/login";
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (inFlight.current || !available || done) return;
    inFlight.current = true;
    try {
      if (!challenge) {
        if (Date.now() < until) return;
        await task.run(async () => {
          try {
            const result = await api.emailCode(
              email.trim().toLowerCase(),
              "password_reset",
              proof ?? {},
            );
            setChallenge({
              ...result,
              expiresAt: Date.now() + result.expires_in * 1000,
            });
            cooldown(result.resend_after);
          } catch (failure) {
            if (failure instanceof ApiRequestError && failure.status === 429)
              cooldown(failure.retryAfterSeconds ?? 60);
            throw failure;
          } finally {
            setProof(undefined);
            setRefreshKey((value) => value + 1);
          }
        });
      } else {
        if (Date.now() < confirmUntil) return;
        await task.run(async () => {
          if (Date.now() >= challenge.expiresAt)
            throw new Error("验证码已过期，请重新获取。");
          if (!/^\d{6}$/.test(code)) throw new Error("请输入 6 位邮箱验证码。");
          if (password !== confirmation)
            throw new Error("两次输入的密码不一致");
          if (
            Array.from(password).length < 8 ||
            new TextEncoder().encode(password).length > 1024
          )
            throw new Error("密码需要至少 8 个字符，且不超过 1024 字节");
          try {
            await api.resetPassword(challenge.challenge_id, code, password);
          } catch (failure) {
            if (failure instanceof ApiRequestError && failure.status === 429)
              setConfirmUntil(
                Date.now() + (failure.retryAfterSeconds ?? 60) * 1000,
              );
            throw failure;
          }
          setPassword("");
          setConfirmation("");
          setCode("");
          setDone(true);
        }, false);
      }
    } finally {
      inFlight.current = false;
    }
  }

  return (
    <form
      className="auth-card security-form"
      onSubmit={submit}
      aria-busy={task.pending}
    >
      <h2>{done ? "密码已重置" : "重置密码"}</h2>
      {!available ? (
        <Notice>本站暂未开放邮箱找回，请联系管理员。</Notice>
      ) : done ? (
        <MessageToast
          kind="success"
          message="密码已重置，旧登录和访问令牌已失效，请重新登录。"
        />
      ) : (
        <>
          <TextField
            label="邮箱"
            type="email"
            required
            maxLength={320}
            disabled={!!challenge || task.pending}
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
                若该邮箱对应可用账号，验证码将很快送达。签发后{" "}
                {challenge.expires_in} 秒内有效。
              </p>
              {expired && <Notice>验证码已过期，请重新获取。</Notice>}
              <TextField
                label="邮箱验证码"
                required
                maxLength={6}
                inputMode="numeric"
                value={code}
                onValue={(value) =>
                  setCode(value.replace(/\D/g, "").slice(0, 6))
                }
                pattern="[0-9]{6}"
                autoComplete="one-time-code"
                disabled={task.pending || expired}
              />
              <TextField
                label="新密码"
                type="password"
                minLength={8}
                maxLength={1024}
                required
                autoComplete="new-password"
                value={password}
                onValue={setPassword}
                disabled={task.pending}
              />
              <TextField
                label="确认新密码"
                type="password"
                minLength={8}
                maxLength={1024}
                required
                autoComplete="new-password"
                value={confirmation}
                onValue={setConfirmation}
                disabled={task.pending}
              />
              <Button
                type="button"
                variant="outline"
                disabled={remaining > 0 || task.pending}
                onClick={editEmail}
              >
                {remaining
                  ? `${remaining} 秒后可重新获取验证码`
                  : "重新获取验证码"}
              </Button>
              <Button
                type="button"
                variant="ghost"
                disabled={task.pending}
                onClick={editEmail}
              >
                修改邮箱
              </Button>
            </>
          )}
          <Button
            type="submit"
            disabled={
              task.pending ||
              expired ||
              (challenge
                ? confirmRemaining > 0
                : remaining > 0 || !proof?.captcha_code)
            }
          >
            {task.pending
              ? "处理中…"
              : challenge
                ? confirmRemaining
                  ? `${confirmRemaining} 秒后可重试`
                  : "重置密码"
                : remaining
                  ? `${remaining} 秒后可发送`
                  : "发送邮箱验证码"}
          </Button>
        </>
      )}
      <a href={login}>返回登录</a>
    </form>
  );
}

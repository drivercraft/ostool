import { MessageToast } from "@/components/message-toast";
import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  api,
  ApiRequestError,
  type EmailChallenge,
  type HumanProof,
} from "@/api/client";
import { useEdition } from "@/edition";
import { parsePasswordResetLink } from "@/utils/password-reset";
import { HumanVerification } from "./human-verification";
import { Button } from "./ui/button";
import { TextField, Notice, useAction } from "./forms";
import { useCountdown } from "./security-hooks";

const cooldownKey = "ostool.password-reset.email-cooldown";
export function PasswordReset() {
  const available = useEdition().password_reset_available;
  const [link, setLink] = useState(() => parsePasswordResetLink(location.hash));
  const [invalidLink] = useState(
    () => !!location.hash && !parsePasswordResetLink(location.hash),
  );
  // Keep the credential in component memory only, including under React StrictMode.
  useEffect(() => {
    if (location.hash)
      history.replaceState(
        history.state,
        "",
        location.pathname + location.search,
      );
  }, []);
  const [email, setEmail] = useState("");
  const [proof, setProof] = useState<HumanProof>();
  const [sent, setSent] = useState<EmailChallenge>();
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [done, setDone] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);
  const [until, setUntil] = useState(() => {
    try {
      return Number(sessionStorage.getItem(cooldownKey)) || 0;
    } catch {
      return 0;
    }
  });
  const [confirmUntil, setConfirmUntil] = useState(0);
  const inFlight = useRef(false);
  const task = useAction();
  const remaining = useCountdown(until);
  const confirmRemaining = useCountdown(confirmUntil);
  const login =
    new URLSearchParams(location.search).get("surface") === "admin"
      ? "/qcl/login"
      : "/login";

  function cooldown(seconds: number) {
    const value = Date.now() + seconds * 1000;
    setUntil(value);
    try {
      sessionStorage.setItem(cooldownKey, String(value));
    } catch {
      /* Storage may be disabled. */
    }
  }
  function requestAgain() {
    setLink(null);
    setSent(undefined);
    setProof(undefined);
    setPassword("");
    setConfirmation("");
    setRefreshKey((value) => value + 1);
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (inFlight.current || done || (!link && !available)) return;
    inFlight.current = true;
    try {
      if (!link) {
        if (sent || Date.now() < until || !proof?.captcha_code) return;
        await task.run(async () => {
          try {
            const result = await api.emailCode(
              email.trim().toLowerCase(),
              "password_reset",
              proof,
            );
            setSent(result);
            cooldown(result.resend_after);
          } catch (failure) {
            if (failure instanceof ApiRequestError && failure.status === 429)
              cooldown(failure.retryAfterSeconds ?? 60);
            throw failure;
          } finally {
            setProof(undefined);
            setRefreshKey((value) => value + 1);
          }
        }, false);
      } else {
        if (Date.now() < confirmUntil) return;
        await task.run(async () => {
          if (password !== confirmation)
            throw new Error("两次输入的密码不一致");
          if (
            Array.from(password).length < 8 ||
            new TextEncoder().encode(password).length > 1024
          )
            throw new Error("密码需要至少 8 个字符，且不超过 1024 字节");
          try {
            await api.resetPassword(link.challenge_id, link.code, password);
          } catch (failure) {
            if (failure instanceof ApiRequestError && failure.status === 429)
              setConfirmUntil(
                Date.now() + (failure.retryAfterSeconds ?? 60) * 1000,
              );
            throw failure;
          }
          setLink(null);
          setPassword("");
          setConfirmation("");
          setDone(true);
        }, false);
      }
    } finally {
      inFlight.current = false;
    }
  }

  return (
    <form className="auth-card" onSubmit={submit} aria-busy={task.pending}>
      <h2>{done ? "密码已重置" : link ? "设置新密码" : "找回密码"}</h2>
      {done ? (
        <MessageToast
          kind="success"
          message="密码已重置，旧登录和访问令牌已失效，请重新登录。"
        />
      ) : link ? (
        <>
          <p>请输入新密码。邮件中的重置链接只能使用一次，过期后请重新申请。</p>
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
            className="auth-submit"
            type="submit"
            disabled={task.pending || confirmRemaining > 0}
          >
            {task.pending
              ? "处理中…"
              : confirmRemaining
                ? `${confirmRemaining} 秒后可重试`
                : "重置密码"}
          </Button>
          <Button
            type="button"
            variant="outline"
            disabled={task.pending}
            onClick={requestAgain}
          >
            重新申请重置邮件
          </Button>
        </>
      ) : !available ? (
        <Notice>
          本站暂未开放邮件找回，请联系管理员配置邮件服务并开启密码找回。
        </Notice>
      ) : sent ? (
        <>
          <Notice>
            若该邮箱对应可用账号，密码重置邮件将很快送达。请点击邮件中的链接设置新密码，链接{" "}
            {sent.expires_in} 秒内有效。
          </Notice>
          <Button
            type="button"
            variant="outline"
            disabled={remaining > 0 || task.pending}
            onClick={requestAgain}
          >
            {remaining ? `${remaining} 秒后可重新发送` : "重新发送重置邮件"}
          </Button>
          <Button
            type="button"
            variant="ghost"
            disabled={task.pending}
            onClick={requestAgain}
          >
            修改邮箱
          </Button>
        </>
      ) : (
        <>
          {invalidLink && <Notice>重置链接格式无效，请重新申请邮件。</Notice>}
          <TextField
            label="注册邮箱"
            type="email"
            required
            maxLength={320}
            autoComplete="email"
            disabled={task.pending}
            value={email}
            onValue={setEmail}
          />
          <HumanVerification
            action="password_reset"
            disabled={task.pending}
            refreshKey={refreshKey}
            onChange={setProof}
          />
          <Button
            className="auth-submit"
            type="submit"
            disabled={task.pending || remaining > 0 || !proof?.captcha_code}
          >
            {task.pending
              ? "处理中…"
              : remaining
                ? `${remaining} 秒后可发送`
                : "发送密码重置邮件"}
          </Button>
        </>
      )}
      {task.error && <MessageToast message={task.error} />}
      <a className="auth-switch" href={login}>
        返回登录
      </a>
    </form>
  );
}

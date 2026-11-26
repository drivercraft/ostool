import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import { Link, useNavigate, useLocation } from "react-router-dom";
import { ArrowRight } from "lucide-react";
import {
  api,
  ApiRequestError,
  notifyAuthChanged,
  type HumanProof,
} from "@/api/client";
import { safeWebsiteRedirect } from "@/auth-session";
import { useEdition } from "@/edition";
import { useSiteSettings } from "@/components/site-settings";
import { AuthLegal } from "@/components/auth-legal";
import { MessageToast } from "@/components/message-toast";
import { FieldTitle, TextField } from "@/components/forms";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { HumanVerification } from "@/components/human-verification";
import { useCountdown } from "@/components/security-hooks";
import { Button } from "@/components/ui/button";
import {
  createRegistrationChallenge,
  canVisitRegistrationStep,
  normalizeRegistrationEmail,
  registrationProgress,
  type RegistrationChallenge,
  type RegistrationStep,
} from "../registration-flow";

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogBody,
} from "@/components/ui/dialog";

const cooldownKey = "ostool.registration.email-cooldown";
export function RegistrationForm() {
  const config = useEdition(),
    site = useSiteSettings(),
    navigate = useNavigate(),
    location = useLocation();
  const emailCodeId = useId();
  const emailRequired = config.registration_email_verification;
  const [step, setStep] = useState<RegistrationStep>(1);
  const [email, setEmail] = useState("");
  const [code, setCode] = useState("");
  const [challenge, setChallenge] = useState<RegistrationChallenge>();
  const [resendOpen, setResendOpen] = useState(false);
  const [resendProof, setResendProof] = useState<HumanProof>();
  const [resendRefreshKey, setResendRefreshKey] = useState(0);
  const [until, setUntil] = useState(() => {
    try {
      return Number(sessionStorage.getItem(cooldownKey)) || 0;
    } catch {
      return 0;
    }
  });
  const remaining = useCountdown(until);
  const [username, setUsername] = useState(""),
    [displayName, setDisplayName] = useState("");
  const [password, setPassword] = useState(""),
    [confirmation, setConfirmation] = useState("");
  const [accepted, setAccepted] = useState(false);
  const [proof, setProof] = useState<HumanProof>();
  const [refreshKey, setRefreshKey] = useState(0);
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const inFlight = useRef(false);
  const form = useRef<HTMLFormElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const progress = registrationProgress(email, code, challenge);
  const needsProof = emailRequired
    ? step === 1 && !progress.active
    : step === 3;
  const hasProof = !!proof?.captcha_code;
  const steps: RegistrationStep[] = emailRequired ? [1, 2, 3] : [1, 3];
  const titles = { 1: "填写邮箱", 2: "验证邮箱", 3: "设置账号" };
  function cooldown(seconds: number) {
    const value = Date.now() + seconds * 1000;
    setUntil(value);
    try {
      sessionStorage.setItem(cooldownKey, String(value));
    } catch {
      /* Storage may be disabled. */
    }
  }
  useEffect(
    () => setAccepted(false),
    [site.terms_version, site.privacy_version],
  );
  useEffect(() => {
    heading.current?.focus();
  }, [step]);
  useEffect(() => {
    if (!emailRequired || !challenge?.expiresAt) return;
    const timer = window.setTimeout(
      () => {
        setChallenge((current) =>
          current ? { ...current, expiresAt: 0, verifiedCode: "" } : current,
        );
        setCode("");
        setStep((current) => (current === 1 ? 1 : 2));
        setError("邮箱验证码已过期，请重新发送。");
      },
      Math.max(0, challenge.expiresAt - Date.now()),
    );
    return () => window.clearTimeout(timer);
  }, [challenge, emailRequired]);
  function visit(target: RegistrationStep) {
    if (
      inFlight.current ||
      !canVisitRegistrationStep(target, emailRequired, email, code, challenge)
    )
      return;
    if (step === 1 && target !== 1 && !form.current?.reportValidity()) return;
    setStep(target);
    setError("");
  }
  function handleUsedEmail(failure: unknown) {
    if (
      !(failure instanceof ApiRequestError) ||
      failure.code !== "registration_email_used"
    )
      return;
    setChallenge(undefined);
    setCode("");
    setResendOpen(false);
    setStep(1);
  }
  async function sendEmail(verification: HumanProof) {
    const result = await api.emailCode(
      normalizeRegistrationEmail(email),
      "register",
      verification,
    );
    setChallenge(createRegistrationChallenge(email, result));
    setCode("");
    cooldown(result.resend_after);
    setStep(2);
  }
  async function resend(event: FormEvent) {
    event.preventDefault();
    event.stopPropagation();
    if (
      inFlight.current ||
      !config.allow_registration ||
      Date.now() < until ||
      !resendProof?.captcha_code
    )
      return;
    inFlight.current = true;
    setBusy(true);
    setError("");
    try {
      await sendEmail(resendProof);
      setResendOpen(false);
    } catch (failure) {
      handleUsedEmail(failure);
      setError(failure instanceof Error ? failure.message : "发送失败，请重试");
      if (failure instanceof ApiRequestError && failure.status === 429)
        cooldown(failure.retryAfterSeconds ?? 60);
    } finally {
      inFlight.current = false;
      setBusy(false);
      setResendProof(undefined);
      setResendRefreshKey((value) => value + 1);
    }
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (inFlight.current || !config.allow_registration) return;
    inFlight.current = true;
    setBusy(true);
    setError("");
    try {
      if (step === 1) {
        if (!emailRequired) {
          setStep(3);
          return;
        }
        if (registrationProgress(email, code, challenge).active) {
          setStep(2);
          return;
        }
        if (Date.now() < until) throw new Error("请等待重发冷却结束后再发送。");
        try {
          await sendEmail(proof ?? {});
        } finally {
          setProof(undefined);
          setRefreshKey((v) => v + 1);
        }
        return;
      }
      if (
        emailRequired &&
        (!registrationProgress(email, code, challenge).active || !challenge)
      ) {
        setStep(2);
        throw new Error("邮箱验证码已失效，请重新发送。");
      }
      if (step === 2 && !emailRequired) {
        setStep(3);
        return;
      }
      if (step === 2 && challenge) {
        if (!/^\d{6}$/.test(code)) throw new Error("请输入 6 位邮箱验证码。");
        await api.verifyRegistrationEmail(
          challenge.email,
          challenge.challenge_id,
          code,
        );
        if (!registrationProgress(email, code, challenge).active)
          throw new Error("邮箱验证码已过期，请重新发送。");
        setChallenge({ ...challenge, verifiedCode: code });
        setStep(3);
        return;
      }
      if (
        emailRequired &&
        !registrationProgress(email, code, challenge).verified
      ) {
        setStep(2);
        throw new Error("请先验证邮箱。");
      }
      if (password !== confirmation) throw new Error("两次输入的密码不一致");
      if (
        Array.from(password).length < 8 ||
        new TextEncoder().encode(password).length > 1024
      )
        throw new Error("密码需要至少 8 个字符，且不超过 1024 字节");
      if (!accepted) throw new Error("请阅读并同意用户协议和隐私政策");
      await api.register(
        username,
        normalizeRegistrationEmail(email),
        password,
        displayName,
        emailRequired ? {} : (proof ?? {}),
        emailRequired && challenge
          ? { id: challenge.challenge_id, code }
          : undefined,
        {
          accepted,
          terms_version: site.terms_version,
          privacy_version: site.privacy_version,
        },
      );
      setPassword("");
      setConfirmation("");
      notifyAuthChanged();
      navigate(
        safeWebsiteRedirect(
          new URLSearchParams(location.search).get("redirect"),
        ),
      );
    } catch (failure) {
      handleUsedEmail(failure);
      setError(failure instanceof Error ? failure.message : "注册失败，请重试");
      if (failure instanceof ApiRequestError) {
        if (failure.code === "registration_email_invalid") {
          setChallenge((current) =>
            current ? { ...current, verifiedCode: "" } : current,
          );
          setStep(2);
        }
        if (failure.code === "legal_changed") setAccepted(false);
        if (failure.status === 429 && step === 1)
          cooldown(failure.retryAfterSeconds ?? 60);
      }
      if (!emailRequired && step === 3) {
        setProof(undefined);
        setRefreshKey((v) => v + 1);
      }
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  }
  return (
    <>
      <form ref={form} className="auth-card" onSubmit={submit} aria-busy={busy}>
        <div>
          <h2 ref={heading} tabIndex={-1}>
            {titles[step]}
          </h2>
          <p>
            {step === 1
              ? "先填写用于登录与找回密码的邮箱。"
              : step === 2
                ? `请输入发送至 ${email} 的验证码。`
                : "设置账号信息，完成注册。"}
          </p>
        </div>
        <ol className="registration-steps" aria-label="注册进度">
          {steps.map((target, index) => (
            <li
              key={target}
              aria-current={step === target ? "step" : undefined}
            >
              <Button
                type="button"
                variant={step === target ? "secondary" : "ghost"}
                disabled={
                  busy ||
                  !canVisitRegistrationStep(
                    target,
                    emailRequired,
                    email,
                    code,
                    challenge,
                  )
                }
                onClick={() => visit(target)}
              >
                {index + 1}. {titles[target]}
              </Button>
            </li>
          ))}
        </ol>
        {!config.allow_registration && (
          <MessageToast
            kind="warning"
            message="当前站点未开放注册，请联系管理员。"
          />
        )}
        <fieldset
          disabled={busy || !config.allow_registration}
          className="registration-fields"
        >
          {step === 1 && (
            <TextField
              label="邮箱"
              type="email"
              required
              maxLength={320}
              autoComplete="email"
              value={email}
              onValue={(value) => {
                setEmail(value);
                setError("");
                if (
                  challenge &&
                  normalizeRegistrationEmail(value) !== challenge.email
                ) {
                  setChallenge(undefined);
                  setCode("");
                }
              }}
              placeholder="you@company.com"
            />
          )}
          {step === 2 && (
            <>
              <Field>
                <FieldTitle label="邮箱验证码" htmlFor={emailCodeId} />
                <div className="input-action">
                  <Input
                    id={emailCodeId}
                    placeholder="请输入邮箱验证码"
                    required={emailRequired}
                    inputMode="numeric"
                    autoComplete="one-time-code"
                    maxLength={6}
                    pattern={emailRequired ? "[0-9]{6}" : undefined}
                    value={code}
                    onChange={(event) => {
                      setCode(
                        event.target.value.replace(/\D/g, "").slice(0, 6),
                      );
                      setChallenge((current) =>
                        current ? { ...current, verifiedCode: "" } : current,
                      );
                      setError("");
                    }}
                    disabled={!progress.active}
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    disabled={remaining > 0}
                    aria-label={
                      remaining
                        ? `${remaining} 秒后可重新发送邮箱验证码`
                        : "重新发送邮箱验证码"
                    }
                    onClick={() => {
                      setResendProof(undefined);
                      setResendOpen(true);
                      setError("");
                    }}
                  >
                    {remaining ? `${remaining} 秒后重发` : "重新发送"}
                  </Button>
                </div>
              </Field>
              {!emailRequired ? (
                <p>站点已取消邮箱验证，可以继续设置账号。</p>
              ) : (
                !progress.active && <p>验证码已过期或失效，请重新发送。</p>
              )}
            </>
          )}
          {step === 3 && (
            <>
              <p>注册邮箱：{normalizeRegistrationEmail(email)}</p>
              {emailRequired && !progress.verified && (
                <p>
                  注册现在要求验证邮箱，请返回第一步完成验证；已填写的账号信息会保留。
                </p>
              )}
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
                maxLength={120}
                value={displayName}
                onValue={setDisplayName}
              />
              <TextField
                label="密码"
                type="password"
                required
                minLength={8}
                maxLength={1024}
                autoComplete="new-password"
                value={password}
                onValue={setPassword}
              />
              <TextField
                label="确认密码"
                type="password"
                required
                minLength={8}
                maxLength={1024}
                autoComplete="new-password"
                value={confirmation}
                onValue={setConfirmation}
              />
            </>
          )}
          {needsProof && (
            <HumanVerification
              action={emailRequired ? "registration_email" : "register"}
              refreshKey={refreshKey}
              onChange={setProof}
              disabled={busy}
            />
          )}
          {step === 3 && (
            <AuthLegal
              registration
              accepted={accepted}
              onChange={setAccepted}
            />
          )}
        </fieldset>
        {error && <MessageToast message={error} />}
        <Button
          className="auth-submit"
          type="submit"
          disabled={
            busy ||
            !config.allow_registration ||
            (needsProof && !hasProof) ||
            (step === 1 && needsProof && remaining > 0) ||
            (step === 2 &&
              emailRequired &&
              (!progress.active || !/^\d{6}$/.test(code))) ||
            (step === 3 && (!accepted || (emailRequired && !progress.verified)))
          }
        >
          {busy
            ? "处理中…"
            : step === 1
              ? needsProof
                ? remaining
                  ? `${remaining} 秒后可发送`
                  : "下一步：发送验证码"
                : "下一步"
              : step === 2
                ? emailRequired
                  ? "验证邮箱并继续"
                  : "下一步"
                : "创建账号"}
          <ArrowRight size={15} />
        </Button>
        {step !== 1 && (
          <Button
            type="button"
            variant="ghost"
            disabled={busy}
            onClick={() =>
              visit(step === 3 && emailRequired && progress.active ? 2 : 1)
            }
          >
            上一步
          </Button>
        )}
        <Link className="auth-switch" to="/login">
          已有账号？返回登录
        </Link>
      </form>
      <Dialog
        open={resendOpen}
        onOpenChange={(open) => {
          if (!busy) setResendOpen(open);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>重新发送验证码</DialogTitle>
            <DialogDescription>
              邮件将发送至 {normalizeRegistrationEmail(email)}
              ；请先完成人机验证。
            </DialogDescription>
          </DialogHeader>
          <DialogBody>
            <form onSubmit={resend}>
              <HumanVerification
                action="registration_email"
                refreshKey={resendRefreshKey}
                onChange={setResendProof}
                disabled={busy}
              />
              <Button
                type="submit"
                disabled={busy || remaining > 0 || !resendProof?.captcha_code}
              >
                {busy
                  ? "发送中…"
                  : remaining > 0
                    ? `${remaining} 秒后可重新发送`
                    : "重新发送邮件"}
              </Button>
            </form>
          </DialogBody>
        </DialogContent>
      </Dialog>
    </>
  );
}

import { useEffect, useState, type FormEvent } from "react";
import { api, type MfaEnrollment, type EmailChallenge } from "@/api/client";
import { security, useSecurityResource, useSecurityErrors } from "@/api/events";
import { Button } from "./ui/button";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "./ui/dialog";
import { TextField, Notice, ConfirmAction, useAction } from "./forms";
import { Skeleton } from "./ui/skeleton";
import { useCountdown } from "./security-hooks";

export function SecuritySettings({
  enrollmentOnly = false,
}: {
  enrollmentOnly?: boolean;
}) {
  const status = useSecurityResource("security"),
    sessions = useSecurityResource("security_sessions"),
    errors = useSecurityErrors();
  const [enrollment, setEnrollment] = useState<MfaEnrollment>(),
    [code, setCode] = useState(""),
    [codes, setCodes] = useState<string[]>(),
    [emailChallenge, setEmailChallenge] = useState<EmailChallenge>(),
    [emailCode, setEmailCode] = useState(""),
    [emailResendAt, setEmailResendAt] = useState(0);
  const emailRemaining = useCountdown(emailResendAt);
  async function sendEmail() {
    await task.run(async () => {
      const result = await api.verifyEmailSend();
      setEmailChallenge(result);
      setEmailCode("");
      setEmailResendAt(Date.now() + result.resend_after * 1000);
    });
  }
  const task = useAction();
  useEffect(() => {
    security.connect();
    return security.disconnect;
  }, []);
  async function begin() {
    await task.run(async () => {
      setEnrollment(await api.enrollMfa());
      setCode("");
    });
  }
  async function confirm(event: FormEvent) {
    event.preventDefault();
    if (!enrollment) return;
    await task.run(async () => {
      const result = await api.confirmMfa(enrollment.challenge_id, code);
      setEnrollment(undefined);
      setCode("");
      setCodes(result.recovery_codes);
    });
  }
  function saved() {
    setCodes(undefined);
    if (enrollmentOnly) window.location.replace("/admin/overview");
  }
  return (
    <div className="security-settings">
      {Object.entries(errors).map(([key, value]) => (
        <Notice key={key}>{value}</Notice>
      ))}
      {task.error && <Notice>{task.error}</Notice>}
      {!status ? (
        <Skeleton className="h-32 w-full" />
      ) : (
        <section className="security-section">
          <h2>双重验证</h2>
          <p>
            {status.enabled
              ? `已启用身份验证器，剩余 ${status.recovery_codes_remaining} 个恢复码。`
              : "启用后，密码登录还需要身份验证器的动态验证码。"}
          </p>
          {!status.available && (
            <Notice>当前服务暂未开放双重验证，请联系管理员。</Notice>
          )}
          {!status.enabled && !enrollment && (
            <Button
              onClick={() => void begin()}
              disabled={task.pending || !status.available}
            >
              设置身份验证器
            </Button>
          )}
          {enrollment && (
            <form onSubmit={confirm} className="mfa-enrollment">
              <p>
                用身份验证器扫描二维码，或手动输入密钥，再填写当前六位验证码。
              </p>
              <img
                className="mfa-qr"
                src={enrollment.qr_image}
                alt="身份验证器设置二维码"
              />
              <code className="mfa-secret">{enrollment.secret}</code>
              <TextField
                label="身份验证器验证码"
                value={code}
                onValue={setCode}
                required
                maxLength={6}
                inputMode="numeric"
                autoComplete="one-time-code"
              />
              <div className="security-actions">
                <Button
                  type="button"
                  variant="outline"
                  disabled={task.pending}
                  onClick={() => setEnrollment(undefined)}
                >
                  取消设置
                </Button>
                <Button type="submit" disabled={task.pending}>
                  确认启用
                </Button>
              </div>
            </form>
          )}
          {status.enabled && (
            <div className="security-actions">
              <Button
                variant="outline"
                disabled={task.pending}
                onClick={() =>
                  void task.run(async () =>
                    setCodes(
                      (await api.regenerateRecoveryCodes()).recovery_codes,
                    ),
                  )
                }
              >
                重新生成恢复码
              </Button>
              {!(
                status.require_admin_mfa &&
                location.pathname.startsWith("/admin")
              ) && (
                <ConfirmAction
                  label="关闭双重验证"
                  description="关闭后，其他登录会话和访问令牌将失效。管理员强制验证策略仍会生效。"
                  action={() => api.disableMfa()}
                />
              )}
            </div>
          )}
        </section>
      )}
      {!enrollmentOnly && status?.email_verification_available && (
        <section className="security-section">
          <h2>邮箱验证</h2>
          <p>
            {status.email_verified
              ? "当前邮箱已验证。"
              : "验证当前邮箱，便于安全恢复账号。"}
          </p>
          {!status.email_verified && !emailChallenge && (
            <Button
              variant="outline"
              disabled={task.pending}
              onClick={() => void sendEmail()}
            >
              发送邮箱验证码
            </Button>
          )}
          {emailChallenge && !status.email_verified && (
            <form
              className="security-form"
              onSubmit={(event) => {
                event.preventDefault();
                void task.run(async () => {
                  await api.verifyEmailConfirm(
                    emailChallenge.challenge_id,
                    emailCode,
                  );
                  setEmailChallenge(undefined);
                  setEmailCode("");
                });
              }}
            >
              <TextField
                label="邮箱验证码"
                required
                maxLength={6}
                inputMode="numeric"
                value={emailCode}
                onValue={setEmailCode}
              />
              <div className="security-actions">
                <Button
                  type="button"
                  variant="outline"
                  disabled={task.pending || emailRemaining > 0}
                  onClick={() => void sendEmail()}
                >
                  {emailRemaining ? emailRemaining + " 秒后可重发" : "重新发送"}
                </Button>
                <Button type="submit" disabled={task.pending}>
                  验证邮箱
                </Button>
              </div>
            </form>
          )}
        </section>
      )}
      {!enrollmentOnly && sessions && (
        <section className="security-section">
          <h2>登录设备</h2>
          <p>
            可撤销网站、管理后台和 CLI
            登录；已签发的个人访问令牌在令牌页面管理。
          </p>
          <ConfirmAction
            label="退出其他设备"
            description="其他登录会话将失效，当前会话保留。"
            action={() => api.revokeOtherSessions()}
          />
          {sessions.sessions.map((session) => (
            <div className="security-session" key={session.id}>
              <div>
                <strong>
                  {session.client}
                  {session.current ? " · 当前会话" : ""}
                </strong>
                <p>
                  {session.ip || "地址未知"} ·{" "}
                  {new Date(session.last_seen_at).toLocaleString()}
                </p>
                <p>{session.user_agent || "未提供设备信息"}</p>
              </div>
              <ConfirmAction
                label="撤销会话"
                description={
                  session.current
                    ? "撤销当前登录并返回登录页面？"
                    : "撤销这个设备的登录？"
                }
                action={async () => {
                  const result = await api.revokeSession(session.id);
                  if (result.current) {
                    await api.logout();
                    window.location.replace(
                      location.pathname.startsWith("/admin")
                        ? "/admin/login"
                        : "/login",
                    );
                  }
                }}
              />
            </div>
          ))}
        </section>
      )}
      <Dialog open={!!codes} onOpenChange={() => {}}>
        <DialogContent showCloseButton={false}>
          <DialogTitle>保存恢复码</DialogTitle>
          <DialogDescription>
            恢复码只展示一次。每个码仅可用一次，请离线妥善保存。
          </DialogDescription>
          <ul className="recovery-codes">
            {codes?.map((code) => (
              <li key={code}>
                <code>{code}</code>
              </li>
            ))}
          </ul>
          <DialogFooter>
            <Button onClick={saved}>已安全保存</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

import { useEffect, useState, type FormEvent } from "react";
import { api, type SecurityStatus } from "@/api/client";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "./ui/dialog";
import { Button } from "./ui/button";
import { TextField, Notice } from "./forms";
type Pending = { resolve: () => void; reject: (error: Error) => void };
export function SecurityVerification() {
  const [pending, setPending] = useState<Pending>(),
    [status, setStatus] = useState<SecurityStatus>(),
    [password, setPassword] = useState(""),
    [code, setCode] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  useEffect(() => {
    const receive = (event: Event) => {
      setPending((event as CustomEvent<Pending>).detail);
      setError("");
      setPassword("");
      setCode("");
      setStatus(undefined);
      void api
        .securityStatus()
        .then(setStatus)
        .catch((e) => setError(e.message));
    };
    window.addEventListener("ostool:step-up", receive);
    return () => window.removeEventListener("ostool:step-up", receive);
  }, []);
  function cancel() {
    if (busy) return;
    pending?.reject(new Error("已取消身份验证"));
    setPending(undefined);
    setPassword("");
    setCode("");
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError("");
    try {
      await api.stepUp(password, code);
      pending?.resolve();
      setPending(undefined);
      setPassword("");
      setCode("");
    } catch (e) {
      setError(e instanceof Error ? e.message : "验证失败");
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog
      open={!!pending}
      onOpenChange={(open) => {
        if (!open) cancel();
      }}
    >
      <DialogContent>
        <DialogTitle>验证当前身份</DialogTitle>
        <DialogDescription>
          验证通过后，当前登录会话可在三分钟内执行敏感操作。
        </DialogDescription>
        <form onSubmit={submit} className="security-form">
          <TextField
            label="当前密码"
            type="password"
            autoComplete="current-password"
            value={password}
            onValue={setPassword}
            maxLength={1024}
            required
          />
          {status?.enabled && (
            <TextField
              label="验证码或恢复码"
              autoComplete="one-time-code"
              value={code}
              onValue={setCode}
              maxLength={128}
              required
            />
          )}
          {error && <Notice>{error}</Notice>}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={busy}
              onClick={cancel}
            >
              取消
            </Button>
            <Button type="submit" disabled={busy || !status}>
              {busy ? "验证中…" : "验证身份"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

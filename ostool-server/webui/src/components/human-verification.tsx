import { useEffect, useRef, useState } from "react";
import {
  api,
  type HumanAction,
  type HumanChallenge,
  type HumanProof,
} from "@/api/client";
import { TextField, Notice } from "./forms";
import { Button } from "./ui/button";

type Turnstile = {
  render: (element: HTMLElement, options: Record<string, unknown>) => string;
  remove: (id: string) => void;
};
let turnstileScript: Promise<void> | undefined;
function loadTurnstile() {
  if (!turnstileScript)
    turnstileScript = new Promise<void>((resolve, reject) => {
      const script = document.createElement("script");
      script.src =
        "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";
      script.async = true;
      script.onload = () => resolve();
      script.onerror = () => {
        turnstileScript = undefined;
        script.remove();
        reject(new Error("人机验证加载失败，请重试"));
      };
      document.head.append(script);
    });
  return turnstileScript;
}
function TurnstileWidget({
  challenge,
  onToken,
}: {
  challenge: HumanChallenge;
  onToken: (value: string) => void;
}) {
  const root = useRef<HTMLDivElement>(null),
    callback = useRef(onToken);
  callback.current = onToken;
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true,
      widget: string | undefined;
    const client = () =>
      (window as unknown as { turnstile?: Turnstile }).turnstile;
    void loadTurnstile()
      .then(() => {
        if (active && root.current)
          widget = client()?.render(root.current, {
            sitekey: challenge.site_key,
            action: challenge.action,
            callback: (token: string) => callback.current(token),
            "expired-callback": () => callback.current(""),
            "error-callback": () => {
              callback.current("");
              setError("人机验证失败，请刷新重试");
            },
          });
      })
      .catch((e) => {
        if (active) setError(e.message);
      });
    return () => {
      active = false;
      if (widget) client()?.remove(widget);
    };
  }, [challenge]);
  return (
    <div>
      <div ref={root} />
      {error && <Notice>{error}</Notice>}
    </div>
  );
}
export function HumanVerification({
  action,
  refreshKey = 0,
  disabled,
  onChange,
}: {
  action: HumanAction;
  refreshKey?: number;
  disabled?: boolean;
  onChange: (proof: HumanProof | undefined) => void;
}) {
  const [challenge, setChallenge] = useState<HumanChallenge>();
  const [answer, setAnswer] = useState(""),
    [error, setError] = useState(""),
    [loading, setLoading] = useState(true),
    [revision, setRevision] = useState(0);
  const changed = useRef(onChange);
  changed.current = onChange;
  useEffect(() => {
    let active = true;
    setLoading(true);
    setAnswer("");
    setChallenge(undefined);
    setError("");
    changed.current(undefined);
    void api
      .captcha(action)
      .then((value) => {
        if (active) setChallenge(value);
      })
      .catch((e) => {
        if (active) setError(e instanceof Error ? e.message : "验证码加载失败");
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [action, refreshKey, revision]);
  useEffect(() => {
    if (!challenge) return;
    const timer = setTimeout(
      () => setRevision((v) => v + 1),
      challenge.expires_in * 1000,
    );
    return () => clearTimeout(timer);
  }, [challenge]);
  const fill = (value: string) => {
    setAnswer(value);
    changed.current(
      challenge && value
        ? { captcha_id: challenge.id, captcha_code: value }
        : undefined,
    );
  };
  return (
    <div className="human-verification" data-captcha-id={challenge?.id}>
      {challenge?.provider === "turnstile" ? (
        <>
          <TurnstileWidget challenge={challenge} onToken={fill} />
          <Button
            type="button"
            variant="outline"
            onClick={() => setRevision((v) => v + 1)}
          >
            重新验证
          </Button>
        </>
      ) : (
        <div className="captcha-row">
          <TextField
            label="图形验证码"
            value={answer}
            onValue={fill}
            placeholder="输入图片中的字符"
            required
            autoComplete="off"
            maxLength={8}
            disabled={disabled || loading}
          />
          <Button
            className="captcha-image"
            type="button"
            variant="outline"
            aria-label="刷新图形验证码"
            title="刷新图形验证码"
            disabled={disabled || loading}
            onClick={() => setRevision((v) => v + 1)}
          >
            {challenge?.image_data_url ? (
              <img
                src={challenge.image_data_url}
                alt="图形验证码"
                width={160}
                height={42}
              />
            ) : loading ? (
              "加载中…"
            ) : (
              "重新加载"
            )}
          </Button>
        </div>
      )}
      {error && (
        <p className="security-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

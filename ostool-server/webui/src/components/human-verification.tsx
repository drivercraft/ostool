import { loadTurnstile, type TurnstileApi } from "@/turnstile";
import { MessageToast } from "@/components/message-toast";
import { useEffect, useId, useRef, useState } from "react";
import {
  api,
  type HumanAction,
  type HumanChallenge,
  type HumanProof,
} from "@/api/client";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Field, FieldLabel } from "./ui/field";

function TurnstileWidget({
  challenge,
  onToken,
  disabled,
}: {
  challenge: HumanChallenge;
  onToken: (value: string) => void;
  disabled?: boolean;
}) {
  const root = useRef<HTMLDivElement>(null),
    callback = useRef(onToken);
  callback.current = onToken;
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true,
      widget: string | undefined,
      api: TurnstileApi | undefined;
    setError("");
    callback.current("");
    const failed = (message: string) => {
      if (active) {
        callback.current("");
        setError(message);
      }
    };
    void loadTurnstile()
      .then((value) => {
        if (!active || !root.current) return;
        api = value;
        widget = value.render(root.current, {
          sitekey: challenge.site_key,
          action: challenge.action,
          cData: challenge.cdata,
          language: (document.documentElement.lang || "zh-cn").toLowerCase(),
          theme: "auto",
          size: "flexible",
          "response-field": false,
          callback: (token: string) => {
            if (active) {
              setError("");
              callback.current(token);
            }
          },
          "expired-callback": () => failed("验证已过期，请重新验证。"),
          "error-callback": () => {
            failed("Cloudflare 人机验证失败，请重新验证。");
            return true;
          },
          "timeout-callback": () => failed("验证超时，请重新验证。"),
          "unsupported-callback": () =>
            failed("当前浏览器不支持 Cloudflare 人机验证，请更换浏览器。"),
        });
      })
      .catch((error) =>
        failed(error instanceof Error ? error.message : "人机验证加载失败"),
      );
    return () => {
      active = false;
      if (widget !== undefined) api?.remove(widget);
    };
  }, [challenge]);
  return (
    <div aria-disabled={disabled}>
      <div
        ref={root}
        aria-label="Cloudflare Turnstile 人机验证"
        style={disabled ? { pointerEvents: "none" } : undefined}
      />
      {error && <MessageToast message={error} />}
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
  const inputId = useId();
  const [challenge, setChallenge] = useState<HumanChallenge>();
  const [answer, setAnswer] = useState(""),
    [error, setError] = useState(""),
    [loading, setLoading] = useState(true),
    [revision, setRevision] = useState(0),
    [expired, setExpired] = useState(false);
  const changed = useRef(onChange);
  changed.current = onChange;
  useEffect(() => {
    let active = true;
    setLoading(true);
    setExpired(false);
    setAnswer("");
    setChallenge(undefined);
    setError("");
    changed.current(undefined);
    void (
      action === "configuration_test"
        ? api.configurationCaptcha()
        : api.captcha(action)
    )
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
    const timer = setTimeout(() => {
      setExpired(true);
      setAnswer("");
      changed.current(undefined);
      setError("验证已过期，请刷新后重试。");
    }, challenge.expires_in * 1000);
    return () => clearTimeout(timer);
  }, [challenge]);
  const fill = (value: string) => {
    setAnswer(value);
    changed.current(
      challenge && !expired && value
        ? { captcha_id: challenge.id, captcha_code: value }
        : undefined,
    );
  };
  return (
    <div className="human-verification" data-captcha-id={challenge?.id}>
      {challenge?.provider === "turnstile" ? (
        <>
          {!expired && (
            <TurnstileWidget
              challenge={challenge}
              onToken={fill}
              disabled={disabled}
            />
          )}
          <Button
            type="button"
            variant="outline"
            disabled={disabled || loading}
            onClick={() => setRevision((v) => v + 1)}
          >
            重新验证
          </Button>
        </>
      ) : (
        <Field className="captcha-field">
          <FieldLabel htmlFor={inputId}>图形验证码</FieldLabel>
          <div className="captcha-input">
            <Input
              id={inputId}
              value={answer}
              onChange={(event) => fill(event.target.value)}
              placeholder="输入图片中的字符"
              required
              autoComplete="off"
              maxLength={8}
              disabled={disabled || loading || expired}
            />
            <Button
              className="captcha-image"
              type="button"
              variant="ghost"
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
        </Field>
      )}
      <MessageToast message={error} />
    </div>
  );
}

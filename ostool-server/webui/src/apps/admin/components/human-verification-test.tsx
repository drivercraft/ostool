import { useEffect, useRef, useState, type FormEvent } from "react";
import { api, type HumanProof } from "@/api/client";
import { HumanVerification } from "@/components/human-verification";
import { Button } from "@/components/ui/button";
import { Section, useAction } from "@/components/forms";

export function HumanVerificationTest({
  revision,
  disabled,
}: {
  revision: number;
  disabled: boolean;
}) {
  const [open, setOpen] = useState(false),
    [proof, setProof] = useState<HumanProof>();
  const [refreshKey, setRefreshKey] = useState(0);
  const task = useAction(),
    inFlight = useRef(false),
    currentRevision = useRef(revision);
  currentRevision.current = revision;
  useEffect(() => {
    setOpen(false);
    setProof(undefined);
  }, [revision]);
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (disabled || inFlight.current || !proof?.captcha_code) return;
    inFlight.current = true;
    const testedRevision = revision;
    try {
      const ok = await task.run(async () => {
        await api.verifyConfigurationCaptcha(proof);
        if (testedRevision !== currentRevision.current)
          throw new Error("配置已更新，请测试最新保存的配置。");
      }, "已保存的人机验证配置测试通过");
      if (ok) setOpen(false);
    } finally {
      inFlight.current = false;
      setProof(undefined);
      setRefreshKey((value) => value + 1);
    }
  }
  return (
    <Section title="测试人机验证">
      <p>
        先保存配置，再完成一次实际验证，检查本地图形验证码或 Cloudflare
        的当前配置。
      </p>
      {open ? (
        <form onSubmit={submit} aria-busy={task.pending}>
          <HumanVerification
            action="configuration_test"
            onChange={setProof}
            refreshKey={refreshKey}
            disabled={disabled || task.pending}
          />
          <Button
            type="submit"
            disabled={disabled || task.pending || !proof?.captcha_code}
          >
            验证当前配置
          </Button>
          <Button
            type="button"
            variant="ghost"
            disabled={task.pending}
            onClick={() => setOpen(false)}
          >
            关闭测试
          </Button>
        </form>
      ) : (
        <Button
          type="button"
          variant="outline"
          disabled={disabled}
          onClick={() => {
            setProof(undefined);
            setOpen(true);
          }}
        >
          测试已保存配置
        </Button>
      )}
    </Section>
  );
}

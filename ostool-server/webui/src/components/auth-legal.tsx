import { useId } from "react";
import { Checkbox } from "./ui/checkbox";
import { useSiteSettings } from "./site-settings";
export function AuthLegal({
  registration = false,
  accepted = false,
  onChange,
}: {
  registration?: boolean;
  accepted?: boolean;
  onChange?: (value: boolean) => void;
}) {
  const id = useId(),
    site = useSiteSettings();
  const links = (
    <>
      <a href="/legal/terms" target="_blank" rel="noopener noreferrer">
        《{site.settings.terms_title}》
      </a>
      和
      <a href="/legal/privacy" target="_blank" rel="noopener noreferrer">
        《{site.settings.privacy_title}》
      </a>
    </>
  );
  return registration ? (
    <div className="auth-legal">
      <Checkbox
        id={id}
        aria-label="我已阅读并同意用户协议和隐私政策"
        checked={accepted}
        onCheckedChange={(value) => onChange?.(value === true)}
      />
      <label className="auth-legal-copy" htmlFor={id}>
        我已阅读并同意{links}
      </label>
    </div>
  ) : (
    <p className="auth-legal">登录前请阅读{links}。</p>
  );
}

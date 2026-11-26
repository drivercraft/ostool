import { useId } from "react";
import { Checkbox } from "./ui/checkbox";
import { useSiteSettings } from "./site-settings";
export function AuthLegal({
  accepted,
  onChange,
  disabled = false,
}: {
  accepted: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
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
  return (
    <div className="auth-legal">
      <Checkbox
        id={id}
        aria-label="我已阅读并同意用户协议和隐私政策"
        required
        disabled={disabled}
        checked={accepted}
        onCheckedChange={(value) => onChange(value === true)}
      />
      <label className="auth-legal-copy" htmlFor={id}>
        我已阅读并同意{links}
      </label>
    </div>
  );
}

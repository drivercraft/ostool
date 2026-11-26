import { splitMailSender, formatMailSender } from "@/apps/admin/mail-sender";
import { useId, useState } from "react";
import { api } from "@/api/client";
import {
  CheckField,
  FormGrid,
  Section,
  TextField,
  SelectField,
  Notice,
  useAction,
} from "@/components/forms";
import { Field } from "@/components/ui/field";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import type {
  AccountSettings,
  AccountSettingsView,
  MailProvider,
  SecretUpdates,
} from "@/types/api";

const providers = [
  {
    id: "brevo",
    label: "Brevo",
    connection: "smtp-relay.brevo.com · 587 · STARTTLS",
    username: "Brevo SMTP 登录名",
    password: "Brevo SMTP Key",
  },
  {
    id: "mailjet",
    label: "Mailjet",
    connection: "in-v3.mailjet.com · 587 · STARTTLS",
    username: "Mailjet API Key",
    password: "Mailjet Secret Key",
  },
  {
    id: "ses",
    label: "Amazon SES",
    connection: "587 · STARTTLS",
    username: "SES SMTP 用户名",
    password: "SES SMTP 密码",
  },
  {
    id: "smtp",
    label: "自部署 SMTP",
    connection: "按服务器配置填写主机、端口与加密方式。",
    username: "SMTP 用户名",
    password: "SMTP 密码",
  },
] as const;
export function MailSettings({
  settings,
  secrets,
  updates,
  onChange,
  onSecretsChange,
  revision,
  disabled,
}: {
  settings: AccountSettings;
  secrets: AccountSettingsView["secrets"];
  updates: SecretUpdates;
  onChange: (key: keyof AccountSettings, value: unknown) => void;
  onSecretsChange: (value: SecretUpdates) => void;
  revision: number;
  disabled: boolean;
}) {
  const id = useId();
  const sender = splitMailSender(settings.smtp_from);
  const test = useAction();
  const [testing, setTesting] = useState<MailProvider>();
  async function testProvider(provider: MailProvider) {
    if (disabled || test.pending) return;
    setTesting(provider);
    await test.run(async () => {
      await api.testMailConnection(
        revision,
        { ...settings, mail_provider: provider },
        updates,
      );
    }, "邮件连接及认证测试通过");
    setTesting(undefined);
  }
  return (
    <>
      <Section
        title="邮件服务"
        hint="各服务商配置独立编辑，保存时一并保存。启用后使用选中的服务发送邮件；敏感凭据留空保留已配置值。"
      >
        <fieldset className="min-w-0 rounded-xl border border-border p-4 pt-2 col-span-full">
          <legend className="px-2 text-sm font-semibold">公共配置</legend>
          <FormGrid columns={3}>
            <CheckField
              label="启用邮件服务"
              checked={settings.smtp_enabled}
              onChange={(value) => onChange("smtp_enabled", value)}
              hint="启用后使用选中的服务商；本地模拟注册邮件独立于 SMTP。"
            />
            <TextField
              label="发件邮箱"
              type="email"
              value={sender.address}
              placeholder="noreply@example.com"
              hint="须通过服务商的发件地址或域名验证。"
              onValue={(value) =>
                onChange("smtp_from", formatMailSender(sender.name, value))
              }
            />
            <TextField
              label="发件名称"
              value={sender.name}
              maxLength={80}
              placeholder="ostool"
              onValue={(value) =>
                onChange("smtp_from", formatMailSender(value, sender.address))
              }
            />
          </FormGrid>
        </fieldset>
      </Section>
      <div
        className="grid min-w-0 grid-cols-1 gap-4 xl:grid-cols-2 mb-6"
        role="group"
        aria-label="邮件服务商"
      >
        {providers.map((provider) => {
          const selected = settings.mail_provider === provider.id;
          const draft = updates.mail_credentials?.[provider.id] ?? {};
          return (
            <fieldset
              key={provider.id}
              className={`min-w-0 rounded-xl border p-4 pt-2 ${selected && settings.smtp_enabled ? "border-primary" : "border-border"}`}
            >
              <legend className="px-2">
                <label className="flex items-center gap-2 text-sm font-semibold">
                  <input
                    type="radio"
                    name={`${id}-provider`}
                    className="size-4 accent-primary"
                    checked={selected}
                    onChange={() => onChange("mail_provider", provider.id)}
                    aria-label={`选择 ${provider.label} 发送邮件`}
                  />
                  {provider.label}
                </label>
              </legend>
              <div className="mb-4 flex min-h-10 items-center justify-between gap-3">
                <p className="text-xs text-muted-foreground break-all">
                  {provider.connection}
                </p>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={disabled || test.pending}
                  onClick={() => void testProvider(provider.id)}
                  title="测试当前填写的配置；留空凭据使用该服务商已保存的值。不会保存配置或发送邮件。"
                >
                  {testing === provider.id ? "测试中…" : "测试"}
                </Button>
              </div>
              <FormGrid columns={2}>
                {provider.id === "ses" && (
                  <>
                    <TextField
                      label="Amazon SES 区域"
                      value={settings.ses_region}
                      placeholder="ap-southeast-2"
                      hint="使用已验证发件人与 SMTP 凭据所在区域。"
                      onValue={(value) => onChange("ses_region", value)}
                    />
                    <TextField
                      label="SES SMTP 端点"
                      value={`email-smtp.${settings.ses_region}.amazonaws.com`}
                      readOnly
                    />
                  </>
                )}
                {provider.id === "smtp" && (
                  <>
                    <TextField
                      label="SMTP 主机"
                      value={settings.smtp_host ?? ""}
                      placeholder="smtp.example.com"
                      onValue={(value) => onChange("smtp_host", value || null)}
                    />
                    <TextField
                      label="SMTP 端口"
                      type="number"
                      min={1}
                      max={65535}
                      step={1}
                      value={settings.smtp_port}
                      onValue={(value) => onChange("smtp_port", Number(value))}
                    />
                    <SelectField
                      label="SMTP 加密方式"
                      value={settings.smtp_tls}
                      onValue={(value) => onChange("smtp_tls", value)}
                      options={[
                        {
                          value: "starttls",
                          label: "STARTTLS（通常为 587 端口）",
                        },
                        { value: "tls", label: "隐式 TLS（通常为 465 端口）" },
                        {
                          value: "loopback_plaintext",
                          label: "回环明文（仅本机）",
                        },
                      ]}
                    />
                  </>
                )}
                {(["username", "password"] as const).map((key) => {
                  const configured = secrets.mail_credentials[provider.id][key];
                  const clearKey = `clear_${key}` as const;
                  const label = provider[key];
                  const clearId = `${id}-${provider.id}-${key}-clear`;
                  return (
                    <Field key={key}>
                      <TextField
                        label={label}
                        type="password"
                        autoComplete="new-password"
                        maxLength={key === "username" ? 320 : 4096}
                        placeholder={
                          configured ? "已配置，留空不修改" : "未配置"
                        }
                        hint="加密保存，仅显示配置状态；填写新值替换原值。"
                        value={draft[key] ?? ""}
                        disabled={!secrets.mfa_key || draft[clearKey]}
                        onValue={(value) =>
                          onSecretsChange({
                            ...updates,
                            mail_credentials: {
                              ...updates.mail_credentials,
                              [provider.id]: { ...draft, [key]: value },
                            },
                          })
                        }
                      />
                      <div className="flex items-center gap-2">
                        <Checkbox
                          id={clearId}
                          checked={!!draft[clearKey]}
                          disabled={!configured}
                          onCheckedChange={(value) =>
                            onSecretsChange({
                              ...updates,
                              mail_credentials: {
                                ...updates.mail_credentials,
                                [provider.id]: {
                                  ...draft,
                                  [key]: undefined,
                                  [clearKey]: value === true,
                                },
                              },
                            })
                          }
                        />
                        <Label htmlFor={clearId}>清除已保存的{label}</Label>
                      </div>
                    </Field>
                  );
                })}
              </FormGrid>
            </fieldset>
          );
        })}
      </div>
      {!secrets.mfa_key && (
        <Notice>保存敏感配置前，请先配置部署主加密密钥。</Notice>
      )}
      <Section title="验证与邮件策略">
        <TextField
          label="全站每小时邮件额度"
          type="number"
          min={1}
          max={100000}
          value={settings.mail_hourly_limit}
          hint="所有用途共用，失败也计入额度。"
          onValue={(value) => onChange("mail_hourly_limit", Number(value))}
        />
        <TextField
          label="密码恢复验证码有效期（秒）"
          type="number"
          min={300}
          max={7200}
          value={settings.password_reset_ttl_secs}
          onValue={(value) =>
            onChange("password_reset_ttl_secs", Number(value))
          }
        />
      </Section>
    </>
  );
}

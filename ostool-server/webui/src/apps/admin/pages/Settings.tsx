import { History } from "lucide-react";
import { RowAction } from "@/components/row-action";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { DataTable, Row, Cell } from "@/components/forms";
import { PagePanel } from "@/components/page-panel";
import { ListToolbar } from "@/components/list-toolbar";
import { useEffect, useId, useState } from "react";
import { api } from "@/api/client";
import { useResource, useList } from "@/api/events";
import type {
  AdminServerConfigResponse,
  TftpConfig,
  TftpStatus,
  SystemTftpdHpaConfig,
} from "@/types/api";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Section,
  TextField,
  SelectField,
  CheckField,
  Notice,
  ReadonlyInfo,
  useAction,
} from "@/components/forms";
export function TftpState({ status }: { status: TftpStatus }) {
  return (
    <>
      <ReadonlyInfo
        entries={{
          Provider: status.provider,
          启用: status.enabled,
          健康: status.healthy,
          目录可写: status.writable,
          根目录: status.root_dir,
          监听地址: status.bind_addr_or_address,
          服务状态: status.service_state,
          "服务器 IP": status.resolved_server_ip,
          子网掩码: status.resolved_netmask,
        }}
      />
      {status.last_error && <Notice>{status.last_error}</Notice>}
    </>
  );
}
export function Tftp() {
  const source = useResource("tftp");
  const status = useResource("tftp_status");
  return source ? (
    <TftpForm source={source} status={status} />
  ) : (
    <Skeleton className="h-40 w-full" />
  );
}
function TftpForm({
  source,
  status,
}: {
  source: TftpConfig;
  status?: TftpStatus;
}) {
  const formId = useId();
  const [config, setConfig] = useState(source),
    [baseline, setBaseline] = useState(JSON.stringify(source));
  const task = useAction(),
    reconcile = useAction();
  const [previousSource, setPreviousSource] = useState<string | null>(null);
  const live = JSON.stringify(source);
  const conflict = live !== baseline && live !== previousSource;
  useEffect(() => {
    if (live === baseline) setPreviousSource(null);
  }, [live, baseline]);
  const patch = (key: string, value: unknown) =>
    setConfig((c) => ({ ...c, [key]: value }));
  function provider(provider: TftpConfig["provider"]) {
    if (provider === config.provider) return;
    setConfig(
      provider === "builtin"
        ? {
            provider,
            enabled: config.enabled,
            root_dir: config.root_dir,
            bind_addr: "0.0.0.0:69",
          }
        : {
            provider,
            enabled: config.enabled,
            root_dir: config.root_dir,
            config_path: "/etc/default/tftpd-hpa",
            service_name: "tftpd-hpa",
            username: "tftp",
            address: ":69",
            options: "-l -s --map-file /etc/ostool-server-tftp.rules",
            manage_config: false,
            reconcile_on_start: false,
          },
    );
  }
  return (
    <PagePanel
      toolbar={
        <ListToolbar
          actions={
            <Button
              type="button"
              variant="outline"
              disabled={reconcile.pending}
              onClick={() =>
                void reconcile.run(
                  () => api.reconcileTftp(),
                  "Reconcile 已执行",
                )
              }
            >
              {reconcile.pending ? "执行中…" : "执行 Reconcile"}
            </Button>
          }
        />
      }
      footer={
        <>
          <span className="panel-status">
            {conflict
              ? "服务器配置已更新，草稿已保留"
              : JSON.stringify(config) !== baseline
                ? "存在未保存的修改"
                : "配置已保存"}
          </span>
          <div className="panel-footer-actions">
            <Button
              type="button"
              variant="outline"
              disabled={task.pending}
              onClick={() => {
                setConfig(source);
                setBaseline(live);
                setPreviousSource(null);
              }}
            >
              取消
            </Button>
            <Button
              type="submit"
              form={formId}
              disabled={task.pending || conflict}
            >
              {task.pending ? "保存中…" : "保存配置"}
            </Button>
          </div>
        </>
      }
    >
      <form
        id={formId}
        onSubmit={(e) => {
          e.preventDefault();
          if (!conflict)
            void task.run(async () => {
              const r = await api.updateTftpConfig(config);
              setPreviousSource(live);
              setConfig(r.tftp);
              setBaseline(JSON.stringify(r.tftp));
            }, "TFTP 配置已保存");
        }}
      >
        {conflict && (
          <Notice>
            服务器配置发生变化，草稿已保留。
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                setConfig(source);
                setBaseline(JSON.stringify(source));
              }}
            >
              载入服务器版本
            </Button>
          </Notice>
        )}
        <Section title="服务配置">
          <SelectField
            label="Provider"
            value={config.provider}
            onValue={(v) => provider(v as TftpConfig["provider"])}
            options={[
              { value: "builtin", label: "内建 TFTP" },
              { value: "system_tftpd_hpa", label: "system_tftpd_hpa" },
            ]}
          />
          <CheckField
            label="启用 TFTP"
            checked={config.enabled}
            onChange={(v) => patch("enabled", v)}
          />
          <TextField
            label="根目录"
            value={config.root_dir}
            onValue={(v) => patch("root_dir", v)}
            required
          />
          {config.provider === "builtin" ? (
            <TextField
              label="绑定地址"
              value={config.bind_addr}
              onValue={(v) => patch("bind_addr", v)}
              required
            />
          ) : (
            <>
              {(
                [
                  ["config_path", "配置文件"],
                  ["service_name", "服务名"],
                  ["username", "运行用户"],
                  ["address", "监听地址"],
                  ["options", "启动选项"],
                ] as const
              ).map(([key, label]) => (
                <TextField
                  key={key}
                  label={label}
                  value={(config as SystemTftpdHpaConfig)[key] ?? ""}
                  onValue={(v) => patch(key, v)}
                />
              ))}
              <CheckField
                label="允许服务端管理配置文件与重启 service"
                checked={config.manage_config}
                onChange={(v) => patch("manage_config", v)}
              />
              <CheckField
                label="启动时自动 reconcile"
                checked={config.reconcile_on_start}
                onChange={(v) => patch("reconcile_on_start", v)}
              />
            </>
          )}
        </Section>
        {task.error && <Notice>{task.error}</Notice>}

        {reconcile.error && <Notice>{reconcile.error}</Notice>}
      </form>
      {status && (
        <section className="page-section">
          <h2>运行状态</h2>
          <TftpState status={status} />
        </section>
      )}
    </PagePanel>
  );
}
export function Server() {
  const source = useResource("server");
  return (
    <>
      {source ? (
        <ServerForm source={source} />
      ) : (
        <Skeleton className="h-40 w-full" />
      )}
    </>
  );
}
function ServerForm({ source }: { source: AdminServerConfigResponse }) {
  const formId = useId();
  const [config, setConfig] = useState(source.editable),
    [baseline, setBaseline] = useState(JSON.stringify(source.editable));
  const networks = useList("network");
  const task = useAction();
  const [previousSource, setPreviousSource] = useState<string | null>(null);
  const live = JSON.stringify(source.editable);
  const conflict = live !== baseline && live !== previousSource;
  useEffect(() => {
    if (live === baseline) setPreviousSource(null);
  }, [live, baseline]);
  return (
    <PagePanel
      footer={
        <>
          <span className="panel-status">
            {conflict
              ? "服务器配置已更新，草稿已保留"
              : JSON.stringify(config) !== baseline
                ? "存在未保存的修改"
                : "配置已保存"}
          </span>
          <div className="panel-footer-actions">
            <Button
              type="button"
              variant="outline"
              disabled={task.pending}
              onClick={() => {
                setConfig(source.editable);
                setBaseline(live);
                setPreviousSource(null);
              }}
            >
              取消
            </Button>
            <Button
              type="submit"
              form={formId}
              disabled={task.pending || conflict}
            >
              {task.pending ? "保存中…" : "保存配置"}
            </Button>
          </div>
        </>
      }
    >
      <form
        id={formId}
        onSubmit={(e) => {
          e.preventDefault();
          if (!conflict)
            void task.run(async () => {
              if (
                Object.values(config.upload_limits).some(
                  (value) => !Number.isInteger(value) || value < 1,
                )
              )
                throw new Error("上传限制必须填写大于零的整数");
              const r = await api.updateServerConfig(config);
              setPreviousSource(live);
              setConfig(r.editable);
              setBaseline(JSON.stringify(r.editable));
            }, "Server 配置已保存");
        }}
      >
        {conflict && (
          <Notice>
            服务器配置发生变化，草稿已保留。
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                setConfig(source.editable);
                setBaseline(JSON.stringify(source.editable));
              }}
            >
              载入服务器版本
            </Button>
          </Notice>
        )}
        <Section title="网络与上传">
          <SelectField
            label="网络接口"
            value={config.network.interface}
            onValue={(v) =>
              setConfig((c) => ({ ...c, network: { interface: v } }))
            }
            options={[
              { value: "", label: "请选择网络接口" },
              ...networks.map((n) => ({ value: n.name, label: n.label })),
            ]}
          />
          <TextField
            label="Session 文件上传上限（MiB）"
            type="number"
            min={1}
            step={1}
            value={config.upload_limits.session_file_max_mib}
            onValue={(v) =>
              setConfig((c) => ({
                ...c,
                upload_limits: {
                  ...c.upload_limits,
                  session_file_max_mib: Number(v),
                },
              }))
            }
          />
          {(
            [
              [
                "session_total_max_mib",
                "会话文件总容量（MiB）",
                "全部文件合计容量，不小于单文件上限",
              ],
              ["session_max_files", "会话最多文件数", "同名覆盖不增加文件数量"],
              [
                "storage_max_mib",
                "全部会话存储上限（MiB）",
                "限制所有会话文件的总容量",
              ],
              [
                "user_total_max_mib",
                "账号文件总容量（MiB）",
                "限制同一账号全部活动会话的文件总量",
              ],
              [
                "user_max_concurrent_uploads",
                "单账号上传并发数",
                "1–32；同一会话同时只允许一个上传",
              ],
              [
                "max_concurrent_uploads",
                "全局上传并发数",
                "1–32；每个会话同时仅允许一个上传",
              ],
              [
                "upload_timeout_secs",
                "上传总时限（秒）",
                "15–3600；连续 15 秒无数据也会终止",
              ],
            ] as const
          ).map(([key, label, placeholder]) => (
            <TextField
              key={key}
              label={label}
              type="number"
              min={1}
              step={1}
              hint=""
              placeholder={placeholder}
              value={config.upload_limits[key]}
              onValue={(v) =>
                setConfig((c) => ({
                  ...c,
                  upload_limits: { ...c.upload_limits, [key]: Number(v) },
                }))
              }
            />
          ))}
        </Section>
        {task.error && <Notice>{task.error}</Notice>}
      </form>
    </PagePanel>
  );
}

// 可编辑业务策略与部署密钥分开；页面始终通过 SSE 接收保存结果。
type PolicyField = {
  key: keyof import("@/types/api").AccountSettings;
  label: string;
  hint: string;
  type?: "check" | "number" | "select";
  min?: number;
  max?: number;
  placeholder?: string;
  options?: { value: string; label: string }[];
};
const policySections: Record<
  string,
  { title: string; fields: PolicyField[] }[]
> = {
  security: [
    {
      title: "账号与注册",
      fields: [
        {
          key: "allow_registration",
          label: "允许站点用户注册",
          type: "check",
          hint: "关闭后保留登录，停止新账号注册和注册邮件签发。",
        },
        {
          key: "password_reset_enabled",
          label: "允许用户找回密码",
          type: "check",
          hint: "需要可用的 SMTP；关闭后隐藏找回入口并拒绝密码恢复。",
        },
        {
          key: "registration_email_verification",
          label: "注册要求邮箱验证",
          type: "check",
          hint: "默认关闭；开启前须完成 SMTP 配置，图形验证码仍然必须验证。",
        },
      ],
    },
    {
      title: "登录与会话",
      fields: [
        {
          key: "login_max_failures",
          label: "登录失败锁定阈值",
          type: "number",
          min: 3,
          max: 20,
          hint: "同一账号和来源触发限制的失败次数，允许 3–20 次。",
        },
        {
          key: "login_lock_secs",
          label: "登录锁定时长（秒）",
          type: "number",
          min: 60,
          max: 86400,
          hint: "60–86400 秒；默认 900 秒，即 15 分钟。现有窗口到期后采用新时长。",
        },
        {
          key: "max_active_sessions",
          label: "每用户每客户端会话上限",
          type: "number",
          min: 1,
          max: 100,
          hint: "网站、后台及 CLI 分别计数；新登录时撤销超额旧会话，允许 1–100 个。",
        },
        {
          key: "access_token_ttl_secs",
          label: "访问令牌有效期（秒）",
          type: "number",
          min: 60,
          max: 86400,
          hint: "60–86400 秒；新签发或刷新的令牌采用此值。",
        },
        {
          key: "refresh_token_ttl_secs",
          label: "登录链有效期（秒）",
          type: "number",
          min: 300,
          max: 31536000,
          hint: "300–31536000 秒；新登录采用此绝对上限，刷新不会延长已有登录链。",
        },
        {
          key: "device_code_ttl_secs",
          label: "CLI 设备授权码有效期（秒）",
          type: "number",
          min: 60,
          max: 1800,
          hint: "60–1800 秒；仅影响新签发的终端登录授权码。",
        },
      ],
    },
    {
      title: "人机验证与双重验证",
      fields: [
        {
          key: "captcha_provider",
          label: "人机验证方式",
          type: "select",
          hint: "网站和后台统一使用；图形验证码无需第三方服务。",
          options: [
            { value: "image", label: "图形验证码" },
            { value: "turnstile", label: "Cloudflare Turnstile" },
          ],
        },
        {
          key: "turnstile_site_key",
          label: "Turnstile 站点密钥",
          hint: "Cloudflare 提供的公开 Site Key；服务端 Secret 通过 .env 配置。",
          placeholder: "0x4AAAA...",
        },
        {
          key: "turnstile_hostnames",
          label: "允许的验证域名",
          hint: "多个域名以英文逗号分隔，不填写协议、端口或通配符，须与 Cloudflare 配置一致。",
          placeholder: "boards.example.com, admin.example.com",
        },
        {
          key: "require_admin_mfa",
          label: "管理员必须启用双重验证",
          type: "check",
          hint: "需要先配置稳定的加密密钥；启用后未绑定验证器的管理员须先完成绑定。",
        },
      ],
    },
  ],
  email: [
    {
      title: "邮件服务",
      fields: [
        {
          key: "smtp_enabled",
          label: "启用邮件服务",
          type: "check",
          hint: "停用后隐藏邮箱验证与找回密码入口；要求注册邮箱验证时不能停用。",
        },
        {
          key: "smtp_host",
          label: "SMTP 主机",
          hint: "仅填写主机名或 IP，不带 smtp:// 或端口。",
          placeholder: "smtp.example.com",
        },
        {
          key: "smtp_port",
          label: "SMTP 端口",
          type: "number",
          min: 1,
          max: 65535,
          hint: "STARTTLS 通常为 587，隐式 TLS 通常为 465。",
        },
        {
          key: "smtp_tls",
          label: "SMTP 传输安全",
          type: "select",
          hint: "公网邮件必须加密；明文仅供回环地址测试或本机邮件代理。",
          options: [
            { value: "starttls", label: "STARTTLS" },
            { value: "tls", label: "隐式 TLS" },
            { value: "loopback_plaintext", label: "回环明文（仅本机）" },
          ],
        },
        {
          key: "smtp_username",
          label: "SMTP 用户名",
          hint: "服务商提供的认证账号，通常为完整邮箱；密码在下方单独设置，更改后保存即可生效。",
          placeholder: "noreply@example.com",
        },
        {
          key: "smtp_from",
          label: "发件人",
          hint: "使用邮箱或“名称 <邮箱>”格式，须符合服务商发件域名要求。",
          placeholder: "ostool <noreply@example.com>",
        },
        {
          key: "password_reset_ttl_secs",
          label: "密码恢复验证码有效期（秒）",
          type: "number",
          min: 300,
          max: 7200,
          hint: "300–7200 秒；仅影响新签发的恢复验证码。",
        },
      ],
    },
  ],
};
export function Accounts({ group }: { group: "security" | "email" }) {
  const source = useResource("account_settings");
  return source ? (
    <AccountForm source={source} group={group} />
  ) : (
    <PagePanel>
      <Skeleton className="h-40 w-full" />
    </PagePanel>
  );
}
function AccountForm({
  source,
  group,
}: {
  source: import("@/types/api").AccountSettingsView;
  group: "security" | "email";
}) {
  const id = useId(),
    task = useAction(),
    deliveries = useList("mail_deliveries");
  const [draft, setDraft] = useState<
    Partial<import("@/types/api").AccountSettings>
  >({});
  const [secretDraft, setSecretDraft] = useState<
    import("@/types/api").SecretUpdates
  >({});
  const [baselineRevision, setBaselineRevision] = useState(source.revision);
  const [baseline, setBaseline] = useState(source.settings),
    [awaiting, setAwaiting] = useState(0),
    [historyOpen, setHistoryOpen] = useState(false);
  const dirty =
    (Object.keys(draft).length > 0 || Object.keys(secretDraft).length > 0) &&
    !awaiting;
  const current = {
    ...source.settings,
    ...(dirty || awaiting > source.revision ? draft : {}),
  };
  const secretDirty = Object.keys(secretDraft).length > 0;
  const conflict =
    (secretDirty && source.revision !== baselineRevision) ||
    (dirty &&
      (Object.keys(draft) as (keyof typeof draft)[]).some(
        (key) =>
          JSON.stringify(source.settings[key]) !==
          JSON.stringify(baseline[key]),
      ));
  useEffect(() => {
    if (awaiting && source.revision >= awaiting) {
      setBaselineRevision(source.revision);
      setDraft({});
      setSecretDraft({});
      setAwaiting(0);
    }
  }, [awaiting, source.revision]);
  const change = (key: keyof typeof current, value: unknown) => {
    if (awaiting) {
      setAwaiting(0);
      setBaseline(current);
    } else if (!dirty) {
      setBaseline(source.settings);
      setBaselineRevision(source.revision);
    }
    setDraft((d) => ({ ...d, [key]: value }));
  };
  return (
    <PagePanel
      footer={
        <>
          <span className="panel-status">
            策略版本 {source.revision}
            {dirty ? " · 尚未保存" : ""} · 保存后生效
          </span>
          <div className="panel-footer-actions">
            <Button
              variant="outline"
              disabled={task.pending}
              onClick={() => {
                setDraft({});
                setSecretDraft({});
                setAwaiting(0);
                setBaseline(source.settings);
                setBaselineRevision(source.revision);
              }}
            >
              取消
            </Button>
            <Button
              type="submit"
              form={id}
              disabled={!dirty || conflict || task.pending}
            >
              {task.pending ? "保存中…" : "保存配置"}
            </Button>
          </div>
        </>
      }
    >
      {conflict && (
        <Notice>
          当前策略已被修改，草稿已保留；请先复制草稿，再取消以载入最新策略。
        </Notice>
      )}
      {task.error && <Notice>{task.error}</Notice>}
      <form
        id={id}
        onSubmit={(event) => {
          event.preventDefault();
          if (!conflict)
            void task.run(async () => {
              await api.updateAccountSettings(
                secretDirty ? baselineRevision : source.revision,
                current,
                secretDraft,
              );
              setSecretDraft({});
              setAwaiting(source.revision + 1);
            }, "策略已保存");
        }}
      >
        <fieldset disabled={task.pending} className="settings-fields">
          {policySections[group].map((section) => (
            <Section key={section.title} title={section.title}>
              {section.fields.map((field) => {
                const value = current[field.key];
                if (field.type === "check")
                  return (
                    <CheckField
                      key={field.key}
                      label={field.label}
                      hint={field.hint}
                      checked={Boolean(value)}
                      onChange={(v) => change(field.key, v)}
                    />
                  );
                if (field.type === "select")
                  return (
                    <SelectField
                      key={field.key}
                      label={field.label}
                      hint={field.hint}
                      value={String(value)}
                      options={field.options!}
                      onValue={(v) => change(field.key, v)}
                    />
                  );
                return (
                  <TextField
                    key={field.key}
                    label={field.label}
                    hint={field.hint}
                    placeholder={field.placeholder}
                    type={field.type === "number" ? "number" : "text"}
                    min={field.min}
                    max={field.max}
                    step={field.type === "number" ? 1 : undefined}
                    value={
                      Array.isArray(value)
                        ? value.join(", ")
                        : String(value ?? "")
                    }
                    onValue={(v) =>
                      change(
                        field.key,
                        field.type === "number"
                          ? Number(v)
                          : field.key === "turnstile_hostnames"
                            ? v
                                .split(",")
                                .map((s) => s.trim())
                                .filter(Boolean)
                            : v || null,
                      )
                    }
                    required={field.type === "number"}
                  />
                );
              })}
            </Section>
          ))}
          <Section
            title={group === "email" ? "邮件认证密码" : "人机验证服务密钥"}
          >
            {(
              [
                group === "email" ? "smtp_password" : "turnstile_secret",
              ] as const
            ).map((key) => (
              <div key={key}>
                <TextField
                  label={
                    key === "smtp_password"
                      ? "SMTP 密码"
                      : "Turnstile 服务端密钥"
                  }
                  type="password"
                  autoComplete="new-password"
                  maxLength={4096}
                  placeholder={
                    source.secrets[key]
                      ? "已配置；留空保留原值，输入新值可替换"
                      : "未配置；输入密钥后保存"
                  }
                  hint="密钥加密存储，不回显原值。"
                  value={secretDraft[key] ?? ""}
                  disabled={
                    !source.secrets.mfa_key || !!secretDraft[`clear_${key}`]
                  }
                  onValue={(value) => {
                    if (!dirty) {
                      setBaseline(source.settings);
                      setBaselineRevision(source.revision);
                    }
                    setSecretDraft((old) => ({ ...old, [key]: value }));
                  }}
                />
                <CheckField
                  label={
                    key === "smtp_password"
                      ? "清除已保存的 SMTP 密码"
                      : "清除已保存的 Turnstile 密钥"
                  }
                  checked={!!secretDraft[`clear_${key}`]}
                  disabled={!source.secrets[key]}
                  onChange={(value) => {
                    if (!dirty) {
                      setBaseline(source.settings);
                      setBaselineRevision(source.revision);
                    }
                    setSecretDraft((old) => ({
                      ...old,
                      [key]: undefined,
                      [`clear_${key}`]: value,
                    }));
                  }}
                />
              </div>
            ))}
            {!source.secrets.mfa_key && (
              <Notice>
                保存敏感配置前，请先配置部署主加密密钥；主密钥不在数据库中保存。
              </Notice>
            )}
          </Section>
          {group === "email" &&
            (
              [
                ["registration_email_policy", "注册邮箱验证码"],
                ["email_policy", "账号邮箱验证与密码恢复"],
              ] as const
            ).map(([key, title]) => (
              <Section title={title} key={key}>
                {(
                  [
                    [
                      "ttl_secs",
                      "验证码有效期（秒）",
                      60,
                      key === "registration_email_policy" ? 3600 : 86400,
                      "仅影响新签发的邮箱验证码；密码恢复使用上方独立有效期。",
                    ],
                    [
                      "resend_secs",
                      "重发冷却（秒）",
                      30,
                      3600,
                      "同一邮箱再次发送的最短间隔，前端倒计时以服务端返回值为准。",
                    ],
                    [
                      "window_secs",
                      "发送统计窗口（秒）",
                      3600,
                      86400,
                      "滚动时间窗口，不按自然日清零；默认 3600 秒。",
                    ],
                    [
                      "email_limit",
                      "单邮箱窗口内发送上限",
                      1,
                      100,
                      "同一邮箱在一个统计窗口内最多签发的验证码数量。",
                    ],
                    [
                      "ip_limit",
                      "单来源 IP 窗口内发送上限",
                      1,
                      5000,
                      "限制同一来源向多个邮箱发信，防止绕过单邮箱额度。",
                    ],
                    [
                      "max_attempts",
                      "验证码最大尝试次数",
                      1,
                      5,
                      "达到上限立即失效；修改不恢复已失效的验证码。",
                    ],
                  ] as const
                ).map(([field, label, min, max, hint]) => (
                  <TextField
                    key={field}
                    label={`${title} · ${label}`}
                    hint={hint}
                    type="number"
                    min={min}
                    max={max}
                    step={1}
                    required
                    value={current[key][field]}
                    onValue={(v) =>
                      change(key, { ...current[key], [field]: Number(v) })
                    }
                  />
                ))}
              </Section>
            ))}
        </fieldset>
      </form>
      {group === "email" && (
        <div className="settings-history-entry">
          <RowAction
            label="邮件投递记录"
            icon={History}
            onClick={() => setHistoryOpen(true)}
          />
          <span className="panel-status">
            最近 100 次投递，不记录正文或验证码
          </span>
        </div>
      )}
      <Dialog open={historyOpen} onOpenChange={setHistoryOpen}>
        <DialogContent className="legal-history-dialog">
          <DialogTitle>邮件投递记录</DialogTitle>
          <DialogDescription>
            最近 100 次投递，仅保留用途、收件人、结果与时间。
          </DialogDescription>
          <div className="legal-history-scroll">
            <DataTable
              layout="table"
              headers={["用途", "收件邮箱", "状态", "时间"]}
              empty={!deliveries.length}
            >
              {deliveries.map((mail) => (
                <Row key={mail.id}>
                  <Cell>{mail.purpose}</Cell>
                  <Cell>{mail.recipient_email}</Cell>
                  <Cell>{mail.status}</Cell>
                  <Cell>{new Date(mail.created_at).toLocaleString()}</Cell>
                </Row>
              ))}
            </DataTable>
          </div>
        </DialogContent>
      </Dialog>
    </PagePanel>
  );
}

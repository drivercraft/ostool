import { MailSettings } from "@/apps/admin/components/mail-settings";
import { Checkbox } from "@/components/ui/checkbox";
import { Field } from "@/components/ui/field";
import { Label } from "@/components/ui/label";
import { HumanVerificationTest } from "@/apps/admin/components/human-verification-test";
import { parseVerificationHostnames } from "@/human-verification";
import { useCountdown } from "@/components/security-hooks";
import { MessageToast } from "@/components/message-toast";
import { usePermissions } from "@/components/permissions";
import { Badge } from "@/components/ui/badge";
import { History } from "lucide-react";
import { RowAction } from "@/components/row-action";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogBody,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { DataTable, Row, Cell } from "@/components/forms";
import { PagePanel, EditorFooter } from "@/components/page-panel";
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
  TextareaField,
  TextField,
  ReloadNotice,
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
      {status.last_error && <MessageToast message={status.last_error} />}
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
  const { can, local } = usePermissions();
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
              permission="server.manage"
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
        <EditorFooter
          status={
            conflict
              ? "服务器配置已更新，草稿已保留"
              : JSON.stringify(config) !== baseline
                ? "存在未保存的修改"
                : ""
          }
        >
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
            permission="server.manage"
            type="submit"
            form={formId}
            disabled={task.pending || conflict}
          >
            {task.pending ? "保存中…" : "保存配置"}
          </Button>
        </EditorFooter>
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
          <ReloadNotice
            onReload={() => {
              setConfig(source);
              setBaseline(JSON.stringify(source));
            }}
          >
            服务器配置发生变化，草稿已保留。
          </ReloadNotice>
        )}
        <fieldset disabled={!can("server.manage")}>
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
              hint={
                local
                  ? "允许设备下载会话文件；网络写入仍被拒绝。"
                  : "在设备内网提供只读下载；公网版必须绑定明确的设备内网 IP，不能使用 0.0.0.0。"
              }
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
        </fieldset>
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
  const { can, local } = usePermissions();
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
        <EditorFooter
          status={
            conflict
              ? "服务器配置已更新，草稿已保留"
              : JSON.stringify(config) !== baseline
                ? "存在未保存的修改"
                : ""
          }
        >
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
            permission="server.manage"
            type="submit"
            form={formId}
            disabled={task.pending || conflict}
          >
            {task.pending ? "保存中…" : "保存配置"}
          </Button>
        </EditorFooter>
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
          <ReloadNotice
            onReload={() => {
              setConfig(source.editable);
              setBaseline(JSON.stringify(source.editable));
            }}
          >
            服务器配置发生变化，草稿已保留。
          </ReloadNotice>
        )}
        <fieldset disabled={!can("server.manage")}>
          <Section title="审计日志保留">
            <TextField
              label="保留天数"
              type="number"
              min={0}
              max={3650}
              step={1}
              required
              value={config.audit_retention_days}
              disabled={!can("audits.manage")}
              hint="0 表示永久保留；1–3650 天。每小时清理任务会读取最新策略。"
              onValue={(value) =>
                setConfig((current) => ({
                  ...current,
                  audit_retention_days: Number(value),
                }))
              }
            />
          </Section>
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
                [
                  "session_max_files",
                  "会话最多文件数",
                  "同名覆盖不增加文件数量",
                ],
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
            )
              .filter(([key]) => !local || !key.startsWith("user_"))
              .map(([key, label, placeholder]) => (
                <TextField
                  key={key}
                  label={label}
                  type="number"
                  min={1}
                  step={1}
                  hint={placeholder}
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
        </fieldset>
      </form>
    </PagePanel>
  );
}

// 可编辑业务策略与部署密钥分开；页面始终通过 SSE 接收保存结果。
type PolicyField = {
  key: keyof import("@/types/api").AccountSettings;
  label: string;
  hint: string;
  type?: "check" | "number" | "select" | "list";
  min?: number;
  max?: number;
  placeholder?: string;
  options?: { value: string; label: string }[];
  fullWidth?: boolean;
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
          key: "dev_registration_codes",
          label: "本地调试注册码",
          type: "check",
          hint: "模拟注册邮件，可在邮件投递记录查看有效注册码，无需 SMTP；找回密码仍需邮件。仅用于本地调试，API 须回环监听。",
        },
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
          hint: "默认关闭；开启前须配置邮件服务或启用本地调试注册码；发送注册邮件时仍需完成人机验证。",
        },
      ],
    },
    {
      title: "登录与会话",
      fields: [
        {
          key: "admin_allowed_networks",
          label: "允许访问后台的网段",
          type: "list",
          hint: "填写 IPv4/IPv6 CIDR，逗号或换行分隔；留空不限制。保存时须包含当前管理员来源，防止锁定后台访问。",
          placeholder: "192.168.1.0/24, 127.0.0.1/32",
        },
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
          hint: "网站和后台统一使用；本地图形验证码由服务器自行生成，无需第三方服务。Cloudflare 请使用 Managed 模式组件。",
          options: [
            { value: "image", label: "本地图形验证码（自定义）" },
            { value: "turnstile", label: "Cloudflare Turnstile" },
          ],
        },
        {
          key: "turnstile_site_key",
          label: "Turnstile 站点密钥",
          hint: "Cloudflare 提供的公开 Site Key；Secret Key 在下方加密保存，保存后可测试验证。",
          placeholder: "0x4AAAA...",
        },
        {
          key: "turnstile_hostnames",
          type: "list",
          fullWidth: true,
          label: "允许的验证域名",
          hint: "多个域名用空格、换行或逗号分隔，不含协议、端口或通配符，须覆盖网站和后台访问域名。",
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
  email: [],
};
function DevelopmentRegistrationCode({
  code,
}: {
  code: import("@/types/api").MailDelivery["development_code"];
}) {
  const remaining = useCountdown(code ? Date.parse(code.expires_at) : 0);
  return code && remaining > 0 ? (
    <span>
      <code>{code.value}</code> · {remaining} 秒
    </span>
  ) : (
    <span>—</span>
  );
}
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
  const { can } = usePermissions();
  const id = useId(),
    task = useAction(),
    deliveries = useList("mail_deliveries");
  const [draft, setDraft] = useState<
    Partial<import("@/types/api").AccountSettings>
  >({});
  const [keywordDraft, setKeywordDraft] = useState<
    Partial<Record<keyof import("@/types/api").NamePolicy, string>>
  >({});
  const [listDraft, setListDraft] = useState<
    Partial<Record<"admin_allowed_networks" | "turnstile_hostnames", string>>
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
      setKeywordDraft({});
      setListDraft({});
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
  const secretKeys = ["turnstile_secret"] as const;
  const secretConfigured = (key: "turnstile_secret") => source.secrets[key];
  const changeSecrets = (updates: import("@/types/api").SecretUpdates) => {
    if (!dirty) {
      setBaseline(source.settings);
      setBaselineRevision(source.revision);
    }
    setSecretDraft(updates);
  };
  return (
    <PagePanel
      footer={
        <EditorFooter
          status={
            <>
              策略版本 {source.revision}
              {dirty ? " · 尚未保存" : ""} · 保存后生效
            </>
          }
        >
          <Button
            variant="outline"
            disabled={task.pending}
            onClick={() => {
              setDraft({});
              setKeywordDraft({});
              setListDraft({});
              setSecretDraft({});
              setAwaiting(0);
              setBaseline(source.settings);
              setBaselineRevision(source.revision);
            }}
          >
            取消
          </Button>
          <Button
            permission="security.manage"
            type="submit"
            form={id}
            disabled={!dirty || conflict || task.pending}
          >
            {task.pending ? "保存中…" : "保存配置"}
          </Button>
        </EditorFooter>
      }
    >
      {conflict && (
        <Notice>
          当前策略已被修改，草稿已保留；请先复制草稿，再取消以载入最新策略。
        </Notice>
      )}
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
        <fieldset
          disabled={task.pending || !can("security.manage")}
          className="settings-fields"
        >
          {group === "email" && (
            <MailSettings
              settings={current}
              secrets={source.secrets}
              updates={secretDraft}
              onChange={change}
              onSecretsChange={changeSecrets}
              revision={secretDirty ? baselineRevision : source.revision}
              disabled={conflict || task.pending || !can("security.manage")}
            />
          )}
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
                    fullWidth={field.fullWidth}
                    type={field.type === "number" ? "number" : "text"}
                    min={field.min}
                    max={field.max}
                    step={field.type === "number" ? 1 : undefined}
                    value={
                      field.type === "list"
                        ? (listDraft[
                            field.key as
                              "admin_allowed_networks" | "turnstile_hostnames"
                          ] ?? (Array.isArray(value) ? value.join(", ") : ""))
                        : String(value ?? "")
                    }
                    onValue={(v) => {
                      if (field.type === "list")
                        setListDraft((current) => ({
                          ...current,
                          [field.key]: v,
                        }));
                      change(
                        field.key,
                        field.type === "number"
                          ? Number(v)
                          : field.type === "list"
                            ? parseVerificationHostnames(v)
                            : v || null,
                      );
                    }}
                    required={field.type === "number"}
                  />
                );
              })}
              {section.title === "账号与注册" &&
                (
                  [
                    ["username_keywords", "用户名禁用关键词"],
                    ["display_name_keywords", "昵称禁用关键词"],
                  ] as const
                ).map(([key, label]) => (
                  <TextareaField
                    key={key}
                    label={label}
                    rows={5}
                    maxLength={16384}
                    placeholder="每行一个或用逗号分隔，最多 128 个，每个不超过 64 字。忽略大小写、全角和分隔符，包含即禁止；留空表示不限制。只检查新名称，已有账号不改名不受影响。"
                    value={
                      keywordDraft[key] ?? current.name_policy[key].join("\n")
                    }
                    onValue={(value) => {
                      setKeywordDraft((old) => ({ ...old, [key]: value }));
                      change("name_policy", {
                        ...current.name_policy,
                        [key]: value
                          .split(/[\n,，]/)
                          .map((word) => word.trim())
                          .filter(Boolean),
                      });
                    }}
                  />
                ))}
            </Section>
          ))}
          {group === "security" && (
            <Section title="人机验证服务密钥" columns={2}>
              {secretKeys.map((key) => (
                <Field key={key} className="gap-3">
                  <TextField
                    status={
                      <Badge
                        variant={
                          secretConfigured(key) ? "success" : "secondary"
                        }
                      >
                        {secretConfigured(key) ? "已配置" : "未配置"}
                      </Badge>
                    }
                    label="Turnstile Secret Key"
                    type="password"
                    autoComplete="new-password"
                    maxLength={4096}
                    placeholder={
                      secretConfigured(key) ? "已配置，留空不修改" : "未配置"
                    }
                    hint="凭据加密存储，保存后不回显；勾选清除并保存可删除。"
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
                  <div className="flex items-center gap-2">
                    <Checkbox
                      id={`${id}-${key}-clear`}
                      checked={!!secretDraft[`clear_${key}`]}
                      disabled={!secretConfigured(key)}
                      onCheckedChange={(value) => {
                        if (!dirty) {
                          setBaseline(source.settings);
                          setBaselineRevision(source.revision);
                        }
                        setSecretDraft((old) => ({
                          ...old,
                          [key]: undefined,
                          [`clear_${key}`]: value === true,
                        }));
                      }}
                    />
                    <Label htmlFor={`${id}-${key}-clear`}>
                      清除已保存的 Turnstile 密钥
                    </Label>
                  </div>
                </Field>
              ))}
              {!source.secrets.mfa_key && (
                <Notice>
                  保存敏感配置前，请先配置部署主加密密钥；主密钥不在数据库中保存。
                </Notice>
              )}
            </Section>
          )}
          {group === "email" &&
            (
              [
                ["registration_email_policy", "注册邮箱验证码（额外限制）"],
                ["email_policy", "所有邮件共用限制与账号邮箱验证"],
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
      {group === "security" && (
        <HumanVerificationTest
          revision={source.revision}
          disabled={
            dirty ||
            task.pending ||
            awaiting > source.revision ||
            !can("security.manage")
          }
        />
      )}
      {group === "email" && source.settings.dev_registration_codes && (
        <Notice>
          本地调试注册码已启用：注册邮件模拟发送，可在“邮件投递记录”查看有效注册码。注册码只保存在当前进程内存中，重启后需重新获取；找回密码仍需邮件服务。
        </Notice>
      )}
      {group === "email" && (
        <div className="settings-history-entry">
          <RowAction
            label="邮件投递记录"
            icon={History}
            onClick={() => setHistoryOpen(true)}
          />
          <span className="panel-status">
            {source.settings.dev_registration_codes
              ? "最近 100 次投递，可查看有效的调试注册码"
              : "最近 100 次投递，不记录正文或验证码"}
          </span>
        </div>
      )}
      <Dialog open={historyOpen} onOpenChange={setHistoryOpen}>
        <DialogContent className="legal-history-dialog">
          <DialogHeader>
            <DialogTitle>邮件投递记录</DialogTitle>
          </DialogHeader>
          <DialogBody>
            <DialogDescription>
              {source.settings.dev_registration_codes &&
                "本地模拟记录可查看有效注册码；只在进程内存保留，用后或过期隐藏。"}
              最近 100 次投递，不保存邮件正文。“SMTP
              已接受”不代表已到达收件箱；发送中断不会自动重试。
            </DialogDescription>
            <div className="legal-history-scroll">
              <DataTable
                layout="table"
                headers={[
                  "用途",
                  "收件邮箱",
                  "状态",
                  ...(source.settings.dev_registration_codes
                    ? ["调试注册码"]
                    : []),
                  "时间",
                ]}
                empty={!deliveries.length}
              >
                {deliveries.map((mail) => (
                  <Row key={mail.id}>
                    <Cell>{mail.purpose}</Cell>
                    <Cell>{mail.recipient_email}</Cell>
                    <Cell>
                      <Badge
                        variant={
                          mail.status === "sent"
                            ? "success"
                            : mail.status === "failed"
                              ? "destructive"
                              : "warning"
                        }
                      >
                        {mail.transport === "development"
                          ? "本地模拟"
                          : {
                              sending: "发送中",
                              sent: "SMTP 已接受",
                              failed: "发送失败",
                            }[mail.status]}
                      </Badge>
                    </Cell>
                    {source.settings.dev_registration_codes && (
                      <Cell>
                        <DevelopmentRegistrationCode
                          code={mail.development_code}
                        />
                      </Cell>
                    )}
                    <Cell>{new Date(mail.created_at).toLocaleString()}</Cell>
                  </Row>
                ))}
              </DataTable>
            </div>
          </DialogBody>
        </DialogContent>
      </Dialog>
    </PagePanel>
  );
}

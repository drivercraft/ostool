import { useEffect, useState } from "react";
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
  return (
    <>
      {source ? (
        <TftpForm source={source} />
      ) : (
        <Skeleton className="h-40 w-full" />
      )}
      {status && (
        <section className="page-section">
          <h2>运行状态</h2>
          <TftpState status={status} />
        </section>
      )}
    </>
  );
}
function TftpForm({ source }: { source: TftpConfig }) {
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
            options: "-l -s -c",
            manage_config: false,
            reconcile_on_start: false,
          },
    );
  }
  return (
    <form
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
      <footer className="form-footer">
        <Button
          type="button"
          variant="outline"
          disabled={reconcile.pending}
          onClick={() =>
            void reconcile.run(() => api.reconcileTftp(), "Reconcile 已执行")
          }
        >
          {reconcile.pending ? "执行中…" : "执行 Reconcile"}
        </Button>
        <Button type="submit" disabled={task.pending || conflict}>
          {task.pending ? "保存中…" : "保存配置"}
        </Button>
      </footer>
      {reconcile.error && <Notice>{reconcile.error}</Notice>}
    </form>
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
    <>
      <section className="page-section">
        <h2>只读信息</h2>
        <ReadonlyInfo
          entries={{
            监听地址: source.readonly.listen_addr,
            数据目录: source.readonly.data_dir,
            板卡目录: source.readonly.board_dir,
            "DTB 目录": source.readonly.dtb_dir,
            "HTTP Boot 公共地址": source.readonly.http_boot_public_base_url,
            "DTB 上传上限": `${source.readonly.dtb_upload_max_mib} MiB`,
          }}
        />
      </section>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (!conflict)
            void task.run(async () => {
              if (
                !Number.isInteger(config.upload_limits.session_file_max_mib) ||
                config.upload_limits.session_file_max_mib < 1
              )
                throw new Error("上传上限必须是大于零的整数 MiB");
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
                upload_limits: { session_file_max_mib: Number(v) },
              }))
            }
          />
        </Section>
        {task.error && <Notice>{task.error}</Notice>}
        <footer className="form-footer">
          <Button type="submit" disabled={task.pending || conflict}>
            {task.pending ? "保存中…" : "保存配置"}
          </Button>
        </footer>
      </form>
    </>
  );
}

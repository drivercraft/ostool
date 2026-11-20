import { useState } from "react";
import { useResource } from "@/api/events";
import { api } from "@/api/client";
import { Button } from "@/components/ui/button";
import { usePermissions } from "@/components/permissions";
import {
  TextField,
  ReloadNotice,
  TextareaField,
  Section,
  Notice,
  useAction,
  ConfirmAction,
} from "@/components/forms";
import type { DeviceConnection as Connection } from "@/types/api";

export function DeviceConnection({ boardId }: { boardId: string }) {
  const details = useResource("board_editor");
  if (
    !details ||
    details.id !== boardId ||
    details.device_connection === undefined
  )
    return <Notice>正在读取设备认证配置…</Notice>;
  return (
    <Editor key={boardId} id={boardId} source={details.device_connection} />
  );
}
function sourceKey(value: Connection | null) {
  return JSON.stringify(
    value ? [value.endpoint, value.ca_pem, value.configured] : null,
  );
}
function Editor({ id, source }: { id: string; source: Connection | null }) {
  const { local } = usePermissions();
  const [endpoint, setEndpoint] = useState(source?.endpoint ?? "");
  const [ca, setCa] = useState(source?.ca_pem ?? "");
  const [token, setToken] = useState("");
  const [baseline, setBaseline] = useState(sourceKey(source));
  const task = useAction();
  const conflict = sourceKey(source) !== baseline;
  return (
    <Section
      title="设备认证"
      columns={2}
      hint={
        source?.configured
          ? "已配置设备连接。凭据留空保留现有凭据。"
          : local
            ? "可通过设备内网 UDP 发现 axloader，也可直接配置 HTTP 地址。"
            : "设备内网支持 axloader HTTP，无需 HTTPS 证书或设备凭据；也可通过 UDP 发现。"
      }
    >
      {conflict && (
        <ReloadNotice
          onReload={() => {
            setEndpoint(source?.endpoint ?? "");
            setCa(source?.ca_pem ?? "");
            setToken("");
            setBaseline(sourceKey(source));
          }}
          label="载入最新配置"
        >
          服务器认证配置已更新，输入已保留。
        </ReloadNotice>
      )}
      <TextField
        label="设备 HTTP / HTTPS 地址"
        fullWidth={false}
        value={endpoint}
        onValue={setEndpoint}
        hint="例如 http://192.168.10.20:8080，只填写来源地址；HTTP 必须使用内网 IP。"
      />
      <TextField
        label="设备独立凭据"
        fullWidth={false}
        value={token}
        type="password"
        onValue={setToken}
        hint="可选，仅设备支持 Bearer 认证时填写，至少 32 个字符；普通 axloader 留空。"
      />
      <TextareaField
        label="设备 CA 证书（可选）"
        value={ca}
        onValue={setCa}
        hint="设备使用私有 CA 时填写 PEM 证书；留空使用系统信任的 CA。"
      />
      <Button
        permission="devices.manage"
        disabled={task.pending || conflict || !endpoint}
        onClick={() =>
          void task.run(async () => {
            const saved = await api.saveDeviceConnection(id, {
              endpoint,
              token: token || undefined,
              ca_pem: ca || null,
            });
            setToken("");
            setBaseline(sourceKey(saved));
          }, "设备认证配置已保存")
        }
      >
        保存设备认证
      </Button>
      {source?.configured && (
        <ConfirmAction
          label="移除设备认证"
          description="移除后该设备不能执行网络启动或升级。请先结束活动会话；公网版还需先禁用开发板。"
          disabled={task.pending || conflict}
          action={async () => {
            await api.removeDeviceConnection(id);
            setEndpoint("");
            setCa("");
            setToken("");
            setBaseline("null");
          }}
        />
      )}
    </Section>
  );
}

import { useState } from "react";
import { useResource } from "@/api/events";
import { api } from "@/api/client";
import { Button } from "@/components/ui/button";
import {
  TextField,
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
  const [endpoint, setEndpoint] = useState(source?.endpoint ?? "");
  const [ca, setCa] = useState(source?.ca_pem ?? "");
  const [token, setToken] = useState("");
  const [baseline, setBaseline] = useState(sourceKey(source));
  const task = useAction();
  const conflict = sourceKey(source) !== baseline;
  return (
    <Section title="设备认证">
      <Notice>
        {source?.configured
          ? "已配置独立设备凭据。留空保留现有凭据。"
          : "设备需支持 HTTPS 和独立凭据；未配置时无法执行网络启动或升级。"}
      </Notice>
      {conflict && (
        <Notice>
          服务器认证配置已更新，输入已保留。
          <Button
            variant="outline"
            onClick={() => {
              setEndpoint(source?.endpoint ?? "");
              setCa(source?.ca_pem ?? "");
              setToken("");
              setBaseline(sourceKey(source));
            }}
          >
            载入最新配置
          </Button>
        </Notice>
      )}
      <TextField
        label="设备 HTTPS 地址"
        value={endpoint}
        onValue={setEndpoint}
        hint="例如 https://loader.example.com:9443，只填写来源地址。"
      />
      <TextField
        label="设备独立凭据"
        value={token}
        type="password"
        onValue={setToken}
        hint="首次配置必须填写至少 32 个字符；凭据需同时配置在设备端。"
      />
      <TextareaField
        label="设备 CA 证书（可选）"
        value={ca}
        onValue={setCa}
        hint="设备使用私有 CA 时填写 PEM 证书；留空使用系统信任的 CA。"
      />
      <Button
        permission="devices.manage"
        disabled={
          task.pending ||
          conflict ||
          !endpoint ||
          (!source?.configured && !token)
        }
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
          description="移除后该设备不能执行网络启动或升级。请先结束设备上的活动会话。"
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

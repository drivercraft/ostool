import type { SerialRuntimeStatus } from "@/types/api";
const phases: Record<SerialRuntimeStatus["phase"], string> = {
  waiting_device: "等待设备",
  verifying: "验证串口身份",
  discovering: "发现串口",
  bound: "串口已绑定",
  recovering: "恢复串口绑定",
  failed: "绑定失败",
  closed: "已释放",
};
export function SerialRuntimeDetails({
  status,
}: {
  status?: SerialRuntimeStatus;
}) {
  const p = status?.parameters;
  return (
    <div aria-label="串口运行态" className="space-y-1 text-sm">
      <p>{status ? phases[status.phase] : "等待会话自动绑定"}</p>
      {status?.port && <p className="mono">{status.port}</p>}
      {p && (
        <p>
          {p.baud_rate} baud · {p.data_bits} 位 · 校验 {p.parity} · 停止位{" "}
          {p.stop_bits} · 流控 {p.flow_control}
        </p>
      )}
      {status?.warning && <p className="text-muted-foreground">{status.warning}</p>}
      {status?.error && <p className="text-destructive">{status.error}</p>}
    </div>
  );
}

import type { AdminSession } from "@/types/api";
import type { Runtime } from "@/api/events";
import { Badge } from "@/components/ui/badge";
import { FieldHelp } from "@/components/forms";
import { SerialRuntimeDetails } from "./serial-runtime";

export function sessionPresentation(session: AdminSession, runtime?: Runtime) {
  const current = runtime?.active_session_id === session.id;
  const releasing =
    session.state === "releasing" ||
    (current &&
      runtime.lease_state !== "using" &&
      runtime.lease_state !== "idle");
  const releaseFailed =
    releasing &&
    current &&
    (runtime.lease_state === "error" || !!runtime.last_release_error);
  if (releaseFailed)
    return {
      state: "release_failed",
      label: "释放失败",
      variant: "destructive" as const,
      releasing,
      error: runtime?.last_release_error,
    };
  if (releasing)
    return {
      state: "releasing",
      label: "释放中",
      variant: "warning" as const,
      releasing,
      error: null,
    };
  if (session.serial_runtime?.phase === "failed")
    return {
      state: "active",
      label: "串口异常",
      variant: "destructive" as const,
      releasing,
      error: session.serial_runtime.error,
    };
  return {
    state: "active",
    label: session.serial_connected ? "使用中" : "占用中",
    variant: session.serial_connected
      ? ("success" as const)
      : ("info" as const),
    releasing,
    error: null,
  };
}

export function SessionStatus({
  session,
  runtime,
  automaticSerial,
}: {
  session: AdminSession;
  runtime?: Runtime;
  automaticSerial: boolean;
}) {
  const status = sessionPresentation(session, runtime);
  return (
    <>
      <div className="flex items-center gap-1">
        <Badge variant={status.variant}>{status.label}</Badge>
        <FieldHelp label="会话状态详情">
          {status.releasing ? (
            <p>会话资源尚未清理完成，开发板暂不可重新分配。</p>
          ) : (
            <>
              <p>终端通道{session.serial_connected ? "已连接" : "未连接"}</p>
              {automaticSerial &&
                session.serial_connected &&
                session.serial_runtime && (
                  <SerialRuntimeDetails status={session.serial_runtime} />
                )}
            </>
          )}
        </FieldHelp>
      </div>
      {!status.releasing && !status.error && (
        <small>终端{session.serial_connected ? "已连接" : "未连接"}</small>
      )}
      {status.error && (
        <small className="text-destructive">{status.error}</small>
      )}
    </>
  );
}

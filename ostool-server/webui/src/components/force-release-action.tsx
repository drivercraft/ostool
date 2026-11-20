import { Unplug } from "lucide-react";
import { api } from "@/api/client";
import { ConfirmAction } from "./forms";

export function ForceReleaseAction({
  sessionId,
  boardId,
}: {
  sessionId: string;
  boardId: string;
}) {
  return (
    <ConfirmAction
      permission="sessions.manage"
      icon={Unplug}
      label="强制释放"
      description={`强制结束 ${boardId} 的会话？这将断开终端并停止清理重试，不等待设备恢复。未确认断电和恢复的开发板将禁用，失败记录仍保留。`}
      action={() => api.deleteSession(sessionId, true)}
    />
  );
}

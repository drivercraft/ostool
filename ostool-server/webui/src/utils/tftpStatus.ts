import type { TftpStatus } from "@/types/api";

export type StatusTone = "good" | "warn" | "danger";

export function describeTftpStatus(status: TftpStatus): {
  tone: StatusTone;
  label: string;
} {
  if (!status.enabled) {
    return { tone: "warn", label: "已禁用" };
  }
  if (!status.healthy) {
    return { tone: "danger", label: "服务异常" };
  }
  if (!status.writable) {
    return { tone: "warn", label: "不可写" };
  }
  return { tone: "good", label: "运行正常" };
}

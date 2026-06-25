import type { SessionRecordState } from "@/types/api";

export interface SessionDisplayStatus {
  label: string;
  tone: "good" | "warn" | "danger" | "neutral";
}

export function getSessionDisplayStatus(state: SessionRecordState | string | null | undefined): SessionDisplayStatus {
  switch (state) {
    case "active":
      return { label: "已连接", tone: "good" };
    case "releasing":
      return { label: "断开中", tone: "warn" };
    case "released":
      return { label: "已断开", tone: "neutral" };
    case "expired":
      return { label: "已超时", tone: "neutral" };
    case "failed":
      return { label: "异常", tone: "danger" };
    default:
      return { label: state || "-", tone: "neutral" };
  }
}

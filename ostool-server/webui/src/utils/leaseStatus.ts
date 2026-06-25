import type { Lease } from "@/types/api";

export interface LeaseDisplayStatus {
  label: string;
  tone: "good" | "warn" | "danger" | "neutral";
  effectiveState: "pending" | "active" | "expired" | "canceled" | "releasing" | "failed";
}

export function getLeaseDisplayStatus(lease: Lease, now = Date.now()): LeaseDisplayStatus {
  if (lease.state === "released") {
    return { label: "已取消", tone: "neutral", effectiveState: "canceled" };
  }
  if (lease.state === "releasing") {
    return { label: "取消中", tone: "warn", effectiveState: "releasing" };
  }
  if (lease.state === "failed") {
    return { label: "失败", tone: "danger", effectiveState: "failed" };
  }

  const start = Date.parse(lease.starts_at);
  const end = Date.parse(lease.expires_at);
  if (Number.isFinite(end) && now >= end) {
    return { label: "已过期", tone: "neutral", effectiveState: "expired" };
  }
  if (lease.state === "expired") {
    return { label: "已过期", tone: "neutral", effectiveState: "expired" };
  }
  if (Number.isFinite(start) && now < start) {
    return { label: "待生效", tone: "warn", effectiveState: "pending" };
  }
  return { label: "生效中", tone: "good", effectiveState: "active" };
}

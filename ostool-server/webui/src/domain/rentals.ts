import type { RentalRequest } from "@/types/api";

export const requestStatusLabels: Record<RentalRequest["status"], string> = {
  pending: "待审核",
  approved: "已通过",
  rejected: "已拒绝",
  cancelled: "已撤回",
};

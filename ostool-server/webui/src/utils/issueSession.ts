import type { IssueSessionPriority, IssueSessionState } from "@/types/api";

export function getIssueStateDisplay(state: IssueSessionState) {
  if (state === "open") {
    return { label: "待处理", tone: "warn" as const };
  }
  if (state === "in_progress") {
    return { label: "处理中", tone: "neutral" as const };
  }
  if (state === "resolved") {
    return { label: "已解决", tone: "good" as const };
  }
  return { label: "已关闭", tone: "neutral" as const };
}

export function getIssuePriorityLabel(priority: IssueSessionPriority) {
  return {
    low: "低",
    normal: "普通",
    high: "高",
    urgent: "紧急",
  }[priority];
}

export function getIssueCategoryLabel(category: string) {
  return {
    general: "一般问题",
    resource: "资源问题",
    lease: "租赁问题",
    session: "会话问题",
    account: "账号问题",
    other: "其他问题",
  }[category] ?? category;
}

import type { AnnouncementKind, AnnouncementStatus } from "@/types/api";

export function getAnnouncementStatusDisplay(status: AnnouncementStatus) {
  if (status === "published") {
    return { label: "已发布", tone: "good" as const };
  }
  if (status === "hidden") {
    return { label: "已隐藏", tone: "neutral" as const };
  }
  return { label: "草稿", tone: "warn" as const };
}

export function getAnnouncementKindLabel(kind: AnnouncementKind) {
  return {
    system: "系统公告",
    activity: "活动公告",
  }[kind];
}

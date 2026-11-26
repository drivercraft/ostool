import type { AllocationPolicy } from "@/types/api";

export function durationWarning(
  start: number,
  end: number,
  policy: AllocationPolicy,
) {
  if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start)
    return null;
  const minutes = (end - start) / 60000;
  if (
    minutes >= policy.min_available_minutes &&
    minutes <= policy.max_duration_minutes
  )
    return null;
  return `所选时长不在免审批范围（${policy.min_available_minutes}–${policy.max_duration_minutes} 分钟）内，提交后将转人工审核。是否继续提交？`;
}

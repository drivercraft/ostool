export function formatLeaseRemaining(expiresAt: string, now = new Date()): string {
  const expires = new Date(expiresAt);
  const deltaMs = expires.getTime() - now.getTime();
  if (Number.isNaN(expires.getTime())) {
    return "未知";
  }
  if (deltaMs <= 0) {
    return "已过期";
  }

  const totalSeconds = Math.floor(deltaMs / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  if (hours > 0) {
    return `${hours}小时 ${minutes}分`;
  }
  if (minutes > 0) {
    return `${minutes}分 ${seconds}秒`;
  }
  return `${seconds}秒`;
}

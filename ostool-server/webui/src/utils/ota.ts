export function sanitizeImageVersion(fileName: string): string | undefined {
  const version = fileName
    .replace(/[^A-Za-z0-9._-]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 96);
  return version || undefined;
}

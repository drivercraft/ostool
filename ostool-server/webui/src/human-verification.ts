import type { HumanAction, HumanChallenge } from "@/api/client";

/** Reject stale/wrong-purpose or incomplete provider responses before enabling submission. */
export function parseHumanChallenge(
  value: unknown,
  action: HumanAction,
): HumanChallenge {
  if (value && typeof value === "object") {
    const data = value as Record<string, unknown>;
    if (
      data.action === action &&
      typeof data.id === "string" &&
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
        data.id,
      ) &&
      typeof data.expires_in === "number" &&
      Number.isInteger(data.expires_in) &&
      data.expires_in > 0 &&
      data.expires_in <= 300
    ) {
      const base = { id: data.id, action, expires_in: data.expires_in };
      if (
        data.provider === "image" &&
        typeof data.image_data_url === "string" &&
        /^data:image\/png;base64,[A-Za-z0-9+/]+=*$/.test(data.image_data_url)
      ) {
        return {
          ...base,
          provider: "image",
          image_data_url: data.image_data_url,
        };
      }
      if (
        data.provider === "turnstile" &&
        typeof data.site_key === "string" &&
        data.site_key.length > 0 &&
        data.site_key.length <= 256 &&
        data.cdata === data.id
      ) {
        return {
          ...base,
          provider: "turnstile",
          site_key: data.site_key,
          cdata: data.id,
        };
      }
    }
  }
  throw new Error("人机验证配置无效，请刷新或联系管理员。");
}
export function parseVerificationHostnames(value: string): string[] {
  return [
    ...new Set(
      value
        .split(/[\s,，]+/)
        .filter(Boolean)
        .map((host) => host.toLowerCase()),
    ),
  ];
}

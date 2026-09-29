import { describe, expect, it } from "vitest";
import { sanitizeImageVersion } from "./ota";

describe("sanitizeImageVersion", () => {
  it("keeps server-safe filename characters", () => {
    expect(sanitizeImageVersion("axloader-v5.efi")).toBe("axloader-v5.efi");
  });

  it("normalizes spaces and non-ASCII filename text", () => {
    expect(sanitizeImageVersion("  测试 axloader v5.efi  ")).toBe("axloader_v5.efi");
  });

  it("omits an empty label and enforces the server limit", () => {
    expect(sanitizeImageVersion("测试固件")).toBeUndefined();
    expect(sanitizeImageVersion(`${"a".repeat(120)}.efi`)).toHaveLength(96);
  });
});

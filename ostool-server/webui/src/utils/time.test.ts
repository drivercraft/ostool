import { describe, expect, it } from "vitest";

import { formatLeaseRemaining } from "./time";

describe("formatLeaseRemaining", () => {
  it("formats future lease in minutes", () => {
    const result = formatLeaseRemaining(
      "2026-01-01T00:05:30Z",
      new Date("2026-01-01T00:00:00Z"),
    );
    expect(result).toBe("5分 30秒");
  });

  it("returns expired for past timestamps", () => {
    expect(
      formatLeaseRemaining("2026-01-01T00:00:00Z", new Date("2026-01-01T00:01:00Z")),
    ).toBe("已过期");
  });
});

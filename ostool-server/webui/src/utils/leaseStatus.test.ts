import { describe, expect, it } from "vitest";

import type { Lease } from "@/types/api";
import { getLeaseDisplayStatus } from "./leaseStatus";

function makeLease(overrides: Partial<Lease> = {}): Lease {
  return {
    id: "lease-1",
    user_id: "user-1",
    session_id: null,
    board_id: "board-1",
    board_type: "rk3568",
    required_tags: [],
    state: "active",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    starts_at: "2026-01-01T10:00:00Z",
    expires_at: "2026-01-01T12:00:00Z",
    released_at: null,
    failure_message: null,
    ...overrides,
  };
}

describe("getLeaseDisplayStatus", () => {
  it("derives pending, active, and expired labels from the lease time window", () => {
    expect(getLeaseDisplayStatus(makeLease(), Date.parse("2026-01-01T09:00:00Z")).label).toBe("待生效");
    expect(getLeaseDisplayStatus(makeLease(), Date.parse("2026-01-01T11:00:00Z")).label).toBe("生效中");
    expect(getLeaseDisplayStatus(makeLease(), Date.parse("2026-01-01T13:00:00Z")).label).toBe("已过期");
  });

  it("shows released leases as canceled", () => {
    expect(getLeaseDisplayStatus(makeLease({ state: "released" })).label).toBe("已取消");
  });
});

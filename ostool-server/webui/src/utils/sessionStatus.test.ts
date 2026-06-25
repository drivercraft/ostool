import { describe, expect, it } from "vitest";

import { getSessionDisplayStatus } from "./sessionStatus";

describe("getSessionDisplayStatus", () => {
  it("maps active connection states to user-facing labels", () => {
    expect(getSessionDisplayStatus("active")).toEqual({ label: "已连接", tone: "good" });
    expect(getSessionDisplayStatus("releasing")).toEqual({ label: "断开中", tone: "warn" });
  });

  it("maps ended connection states to user-facing labels", () => {
    expect(getSessionDisplayStatus("released")).toEqual({ label: "已断开", tone: "neutral" });
    expect(getSessionDisplayStatus("expired")).toEqual({ label: "已超时", tone: "neutral" });
    expect(getSessionDisplayStatus("failed")).toEqual({ label: "异常", tone: "danger" });
  });
});

import { describe, expect, it } from "vitest";

import type { BoardConfig } from "@/types/api";

import { normalizeBoardTypeForId, suggestBoardId } from "./boardIdentity";

function board(id: string, boardType: string): BoardConfig {
  return {
    id,
    name: id,
    board_type: boardType,
    tags: [],
    serial: null,
    boot: { kind: "pxe", notes: null },
    notes: null,
    disabled: false,
  };
}

describe("boardIdentity", () => {
  it("normalizes board type into an id-friendly prefix", () => {
    expect(normalizeBoardTypeForId(" RK3568 EVB / LAB ")).toBe("rk3568-evb-lab");
  });

  it("suggests the next numeric suffix from existing ids", () => {
    expect(
      suggestBoardId("rk3568", [
        board("rk3568-1", "rk3568"),
        board("rk3568-3", "rk3568"),
        board("visionfive-1", "visionfive"),
      ]),
    ).toBe("rk3568-4");
  });

  it("falls back to count-based numbering when ids do not match pattern", () => {
    expect(
      suggestBoardId("demo", [board("custom-a", "demo"), board("custom-b", "demo")]),
    ).toBe("demo-3");
  });
});

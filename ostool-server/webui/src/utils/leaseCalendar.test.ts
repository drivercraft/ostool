import { describe, expect, it } from "vitest";

import { selectLeaseCalendarRange, type LeaseCalendarSlotBase } from "./leaseCalendar";

function makeSlots(count: number): LeaseCalendarSlotBase[] {
  return Array.from({ length: count }, (_, index) => ({
    startIso: `2026-01-01T${`${index}`.padStart(2, "0")}:00:00.000Z`,
    endIso: `2026-01-01T${`${index + 1}`.padStart(2, "0")}:00:00.000Z`,
  }));
}

describe("selectLeaseCalendarRange", () => {
  it("clears the clicked selected slot and all selected slots after it", () => {
    const slots = makeSlots(6);
    const result = selectLeaseCalendarRange(
      slots,
      slots[2],
      slots[0].startIso,
      slots[4].endIso,
      () => true,
    );

    expect(result).toEqual({
      startIso: slots[0].startIso,
      endIso: slots[1].endIso,
    });
  });

  it("clears the full selection when clicking the first selected slot", () => {
    const slots = makeSlots(3);
    const result = selectLeaseCalendarRange(
      slots,
      slots[0],
      slots[0].startIso,
      slots[2].endIso,
      () => true,
    );

    expect(result).toEqual({ startIso: "", endIso: "" });
  });
});

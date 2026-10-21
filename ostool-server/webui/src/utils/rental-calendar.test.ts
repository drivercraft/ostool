import { expect, it } from "vitest";
import {
  busyWindows,
  calendarDaySlots,
  calendarSelection,
  freeWindows,
  rentalConflicts,
  type RentalWindow,
} from "./rental-calendar";
const row = (
  id: string,
  start: number,
  end: number,
  status = "active",
): RentalWindow => ({
  id,
  board_id: "board",
  starts_at: new Date(start).toISOString(),
  ends_at: new Date(end).toISOString(),
  status,
});
it("merges overlaps and adjacency but keeps exact half-open free windows", () => {
  const bookings = [
    row("a", 20, 40),
    row("b", 35, 60),
    row("c", 60, 70),
    row("cancelled", 75, 95, "cancelled"),
    row("previous", -20, 10),
    row("next", 90, 120),
  ];
  expect(busyWindows(bookings, 0, 100)).toEqual([
    { start: 0, end: 10 },
    { start: 20, end: 70 },
    { start: 90, end: 100 },
  ]);
  expect(freeWindows(bookings, 0, 100)).toEqual([
    { start: 10, end: 20 },
    { start: 70, end: 90 },
  ]);
  expect(rentalConflicts(bookings, 70, 90)).toHaveLength(0);
  expect(rentalConflicts(bookings, 69, 90).map((r) => r.id)).toEqual(["c"]);
});
it("excludes the rental being edited and handles a 25 hour daylight-saving day", () => {
  const start = Date.parse("2026-11-01T00:00:00-04:00"),
    end = Date.parse("2026-11-02T00:00:00-05:00");
  expect(freeWindows([], start, end)[0].end - start).toBe(25 * 3600000);
  expect(
    freeWindows([row("edited", start, end)], start, end, "edited"),
  ).toEqual([{ start, end }]);
  expect(
    freeWindows(
      [row("cross-day", start - 3600000, start + 3600000)],
      start,
      end,
    ),
  ).toEqual([{ start: start + 3600000, end }]);
});
it("keeps PostgreSQL microsecond ends occupied until the next selectable millisecond", () => {
  const end = Date.parse("2026-10-21T10:00:00.123Z");
  const booking = {
    ...row("micro", end - 1000, end),
    ends_at: "2026-10-21T10:00:00.123456Z",
  };
  expect(freeWindows([booking], end - 1000, end + 1000)).toEqual([
    { start: end + 1, end: end + 1000 },
  ]);
  expect(rentalConflicts([booking], end, end + 1)).toHaveLength(1);
  expect(rentalConflicts([booking], end + 1, end + 1000)).toHaveLength(0);
});

it("builds continuous local-day slots through daylight-saving boundaries", () => {
  for (const day of [new Date(2026, 2, 8), new Date(2026, 10, 1)]) {
    const end = new Date(
      day.getFullYear(),
      day.getMonth(),
      day.getDate() + 1,
    ).getTime();
    const slots = calendarDaySlots(day);
    expect(slots[0].start).toBe(day.getTime());
    expect(slots.at(-1)?.end).toBe(end);
    expect(slots.length).toBe((end - day.getTime()) / (30 * 60000));
    for (let i = 1; i < slots.length; i++)
      expect(slots[i].start).toBe(slots[i - 1].end);
    expect(calendarSelection(slots[2], slots[0])).toEqual({
      start: slots[0].start,
      end: slots[2].end,
    });
  }
});

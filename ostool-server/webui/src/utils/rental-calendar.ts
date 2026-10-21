export interface RentalWindow {
  id: string;
  board_id: string;
  starts_at: string;
  ends_at: string;
  status: string;
  label?: string;
}
export interface TimeWindow {
  start: number;
  end: number;
}
// PostgreSQL may return microseconds. A selectable free window must start
// after the exact reservation end, never at JavaScript's truncated millisecond.
function occupiedEnd(value: string) {
  const end = Date.parse(value);
  const fractional = /\.(\d{4,})/.exec(value)?.[1];
  return end + (fractional && /[1-9]/.test(fractional.slice(3)) ? 1 : 0);
}
export function busyWindows(
  bookings: RentalWindow[],
  start: number,
  end: number,
  exclude?: string,
): TimeWindow[] {
  const windows = bookings
    .filter((r) => r.id !== exclude && r.status !== "cancelled")
    .map((r) => ({
      start: Math.max(start, Date.parse(r.starts_at)),
      end: Math.min(end, occupiedEnd(r.ends_at)),
    }))
    .filter(
      (r) =>
        Number.isFinite(r.start) && Number.isFinite(r.end) && r.start < r.end,
    )
    .sort((a, b) => a.start - b.start);
  const merged: TimeWindow[] = [];
  for (const window of windows) {
    const previous = merged.at(-1);
    if (previous && window.start <= previous.end)
      previous.end = Math.max(previous.end, window.end);
    else merged.push({ ...window });
  }
  return merged;
}
export function freeWindows(
  bookings: RentalWindow[],
  start: number,
  end: number,
  exclude?: string,
): TimeWindow[] {
  const result: TimeWindow[] = [];
  let cursor = start;
  for (const busy of busyWindows(bookings, start, end, exclude)) {
    if (cursor < busy.start) result.push({ start: cursor, end: busy.start });
    cursor = busy.end;
  }
  if (cursor < end) result.push({ start: cursor, end });
  return result;
}
export function rentalConflicts(
  bookings: RentalWindow[],
  start: number,
  end: number,
  exclude?: string,
) {
  return bookings.filter(
    (r) =>
      r.id !== exclude &&
      r.status !== "cancelled" &&
      Date.parse(r.starts_at) < end &&
      occupiedEnd(r.ends_at) > start,
  );
}
export function localDateTime(value: Date) {
  return new Date(value.getTime() - value.getTimezoneOffset() * 60000)
    .toISOString()
    .slice(0, 23);
}

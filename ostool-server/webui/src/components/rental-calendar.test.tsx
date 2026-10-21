import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RentalCalendar } from "./rental-calendar";
import { localDateTime, type RentalWindow } from "@/utils/rental-calendar";

class Pointer extends MouseEvent {
  pointerId: number;
  constructor(
    type: string,
    init: MouseEventInit & { pointerId?: number } = {},
  ) {
    super(type, init);
    this.pointerId = init.pointerId ?? 1;
  }
}
beforeEach(() => {
  vi.stubGlobal("PointerEvent", Pointer);
  HTMLElement.prototype.setPointerCapture = vi.fn();
  HTMLElement.prototype.releasePointerCapture = vi.fn();
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  delete (HTMLElement.prototype as Partial<HTMLElement>).setPointerCapture;
  delete (HTMLElement.prototype as Partial<HTMLElement>).releasePointerCapture;
});
const day = new Date(2035, 4, 10);
function setup(bookings: RentalWindow[] = [], excludeId?: string) {
  const onSelect = vi.fn();
  const result = render(
    <RentalCalendar
      bookings={bookings}
      startsAt={localDateTime(day)}
      excludeId={excludeId}
      onSelect={onSelect}
    />,
  );
  return { ...result, onSelect };
}
it("navigates year/month/day without changing the draft, and supports keyboard time selection", async () => {
  const { onSelect } = setup();
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "年视图" }));
  expect(screen.getByRole("button", { name: "12 月" })).toBeTruthy();
  await user.selectOptions(
    screen.getByRole("combobox", { name: "显示年份" }),
    "2036",
  );
  await user.click(screen.getByRole("button", { name: "月视图" }));
  await user.selectOptions(
    screen.getByRole("combobox", { name: "显示月份" }),
    "5",
  );
  expect(onSelect).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "日视图" }));
  const slot = screen.getByRole("button", {
    name: "09:00–09:30，空闲",
  });
  slot.focus();
  await user.keyboard("{Enter}");
  expect(onSelect).toHaveBeenCalledWith(
    new Date(2036, 5, 10, 9),
    new Date(2036, 5, 10, 9, 30),
  );
});
it("selects an inclusive date range in either drag direction, and cancels with Escape", () => {
  const { container, onSelect } = setup();
  const root = screen.getByRole("region", { name: "租赁日历" });
  const first = container.querySelector<HTMLElement>(
    `[data-calendar-start="${day.getTime()}"]`,
  )!;
  const last = container.querySelector<HTMLElement>(
    `[data-calendar-start="${new Date(2035, 4, 12).getTime()}"]`,
  )!;
  Object.defineProperty(document, "elementFromPoint", {
    configurable: true,
    value: vi.fn(() => first),
  });
  fireEvent.pointerDown(last, { button: 0, pointerId: 1 });
  fireEvent.pointerMove(root, { clientX: 1, clientY: 1, pointerId: 1 });
  fireEvent.pointerUp(root, { pointerId: 1 });
  expect(onSelect).toHaveBeenCalledWith(day, new Date(2035, 4, 13));
  onSelect.mockClear();
  fireEvent.pointerDown(first, { button: 0, pointerId: 2 });
  fireEvent.keyDown(root, { key: "Escape" });
  fireEvent.pointerUp(root, { pointerId: 2 });
  expect(onSelect).not.toHaveBeenCalled();
});
it("rejects occupied time ranges without changing the draft and excludes the edited rental", async () => {
  const booking: RentalWindow = {
    id: "busy",
    board_id: "board",
    starts_at: new Date(2035, 4, 10, 9).toISOString(),
    ends_at: new Date(2035, 4, 10, 10).toISOString(),
    status: "scheduled",
  };
  const { onSelect, rerender } = setup([booking]);
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "日视图" }));
  screen.getByRole("button", { name: "09:00–09:30，已有租赁" }).focus();
  await user.keyboard("{Enter}");
  expect(onSelect).not.toHaveBeenCalled();
  expect(screen.getByRole("alert").textContent).toContain("已有租赁");
  rerender(
    <RentalCalendar
      bookings={[booking]}
      startsAt={localDateTime(day)}
      excludeId="busy"
      onSelect={onSelect}
    />,
  );
  screen.getByRole("button", { name: "09:00–09:30，空闲" }).focus();
  await user.keyboard("{Enter}");
  expect(onSelect).toHaveBeenCalledWith(
    new Date(2035, 4, 10, 9),
    new Date(2035, 4, 10, 9, 30),
  );
});

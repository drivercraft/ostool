import { useEffect, useRef, useState, type PointerEvent } from "react";
import {
  ChevronLeft,
  ChevronRight,
  CalendarDays,
  ChevronDown,
} from "lucide-react";
import { Button } from "./ui/button";
import { NativeSelect } from "./ui/native-select";
import {
  calendarDaySlots,
  calendarSelection,
  freeWindows,
  rentalConflicts,
  type RentalWindow,
  type TimeWindow,
} from "@/utils/rental-calendar";

type View = "year" | "month" | "day";
const dateOnly = (value: Date) =>
  new Date(value.getFullYear(), value.getMonth(), value.getDate());
const sameDay = (a: Date, b: Date) =>
  dateOnly(a).getTime() === dateOnly(b).getTime();
const dayEnd = (value: Date) =>
  new Date(value.getFullYear(), value.getMonth(), value.getDate() + 1);
const clock = (value: number, showZone = false) =>
  new Date(value).toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZoneName: showZone ? "short" : undefined,
  });

export function RentalCalendar({
  bookings,
  startsAt,
  endsAt,
  excludeId,
  onSelect,
  onRangeChange,
  disabled = false,
}: {
  bookings: RentalWindow[];
  onRangeChange?: (from: Date, to: Date) => void;
  disabled?: boolean;
  startsAt: string;
  endsAt?: string;
  excludeId?: string;
  onSelect: (start: Date, end: Date) => void;
}) {
  const initial = new Date(startsAt);
  const valid = Number.isNaN(initial.getTime()) ? new Date() : initial;
  const [selected, setSelected] = useState(() => dateOnly(valid));
  const [anchor, setAnchor] = useState(() => dateOnly(valid));
  const [view, setView] = useState<View>("month");
  const [preview, setPreview] = useState<TimeWindow>();
  const [error, setError] = useState("");
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30000);
    return () => clearInterval(timer);
  }, []);
  const cutoff = Math.ceil(now / 60000) * 60000;
  const root = useRef<HTMLElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const drag = useRef<{
    pointer: number;
    first: TimeWindow;
    last: TimeWindow;
  } | null>(null);
  useEffect(() => {
    const date = new Date(startsAt);
    if (!Number.isNaN(date.getTime())) {
      setSelected(dateOnly(date));
      setAnchor(dateOnly(date));
    }
  }, [startsAt]);
  const anchorTime = anchor.getTime();
  useEffect(() => {
    const first = new Date(anchor.getFullYear(), anchor.getMonth(), 1);
    const gridStart = new Date(first);
    gridStart.setDate(1 - ((first.getDay() + 6) % 7));

    const start =
      view === "year"
        ? new Date(anchor.getFullYear(), 0, 1)
        : view === "month"
          ? gridStart
          : dateOnly(anchor);
    const end =
      view === "year"
        ? new Date(anchor.getFullYear() + 1, 0, 1)
        : view === "month"
          ? new Date(
              gridStart.getFullYear(),
              gridStart.getMonth(),
              gridStart.getDate() + 42,
            )
          : dayEnd(anchor);
    onRangeChange?.(start, end);
  }, [view, anchorTime, onRangeChange]);
  const today = new Date();
  const slots = calendarDaySlots(anchor);
  const showSlotZone = slots.length !== 48;
  const start = selected.getTime(),
    end = dayEnd(selected).getTime();
  const reservations = rentalConflicts(bookings, start, end, excludeId).sort(
    (a, b) => Date.parse(a.starts_at) - Date.parse(b.starts_at),
  );
  const free = freeWindows(bookings, Math.max(start, cutoff), end, excludeId);
  const available = (window: TimeWindow) =>
    freeWindows(
      bookings,
      Math.max(window.start, cutoff),
      window.end,
      excludeId,
    );
  const unavailable = (window: TimeWindow) =>
    disabled ||
    window.end <= cutoff ||
    (view === "day"
      ? rentalConflicts(bookings, window.start, window.end, excludeId).length >
        0
      : available(window).length === 0);
  const range =
    preview ??
    (endsAt
      ? { start: Date.parse(startsAt), end: Date.parse(endsAt) }
      : undefined);
  const selectedRange = (window: TimeWindow) =>
    !unavailable(window) &&
    !!range &&
    window.start < range.end &&
    window.end > range.start;

  function select(window: TimeWindow) {
    if (disabled) return;
    const from = Math.max(window.start, Math.ceil(Date.now() / 60000) * 60000);
    if (from >= window.end) {
      setError("请选择尚未结束的时段。");
      return;
    }
    if (rentalConflicts(bookings, from, window.end, excludeId).length) {
      setError("所选时段已有租赁，请调整范围或选择当日可用时段。");
      return;
    }
    setError("");
    onSelect(new Date(from), new Date(window.end));
  }
  function selectSingle(window: TimeWindow) {
    if (disabled) return;
    if (unavailable(window)) return;
    const choice = view === "day" ? window : available(window)[0];
    if (choice) select(choice);
    if (view === "year") {
      setAnchor(new Date(window.start));
      setView("day");
    }
  }
  function cellAt(x: number, y: number): TimeWindow | undefined {
    const cell = document
      .elementFromPoint(x, y)
      ?.closest<HTMLElement>("[data-calendar-start]");
    if (!cell || !root.current?.contains(cell)) return;
    return {
      start: Number(cell.dataset.calendarStart),
      end: Number(cell.dataset.calendarEnd),
    };
  }
  function pointerDown(event: PointerEvent<HTMLElement>) {
    if (event.button !== 0) return;
    const cell = (event.target as HTMLElement).closest<HTMLElement>(
      "[data-calendar-start]",
    );
    if (!cell || cell.matches(":disabled")) return;
    event.preventDefault();
    cell.focus({ preventScroll: true });
    const window = {
      start: Number(cell.dataset.calendarStart),
      end: Number(cell.dataset.calendarEnd),
    };
    drag.current = { pointer: event.pointerId, first: window, last: window };
    setSelected(dateOnly(new Date(window.start)));
    setPreview(window);
    setError("");
    event.currentTarget.setPointerCapture(event.pointerId);
  }
  function pointerMove(event: PointerEvent<HTMLElement>) {
    if (drag.current?.pointer !== event.pointerId) return;
    const cell = cellAt(event.clientX, event.clientY);
    if (cell) {
      drag.current.last = cell;
      setPreview(calendarSelection(drag.current.first, cell));
    }
    const box = body.current?.getBoundingClientRect();
    if (box && event.clientX >= box.left && event.clientX <= box.right) {
      if (event.clientY > box.bottom - 12) body.current?.scrollBy(0, 16);
      else if (event.clientY < box.top + 12) body.current?.scrollBy(0, -16);
    }
  }
  function cancel() {
    drag.current = null;
    setPreview(undefined);
  }
  function pointerUp(event: PointerEvent<HTMLElement>) {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    const window = calendarSelection(current.first, current.last);
    const single = current.first.start === current.last.start;
    cancel();
    event.currentTarget.releasePointerCapture(event.pointerId);
    if (single) selectSingle(window);
    else select(window);
  }
  function navigate(direction: number) {
    cancel();
    if (view === "year")
      setAnchor(
        new Date(anchor.getFullYear() + direction, anchor.getMonth(), 1),
      );
    else if (view === "month")
      setAnchor(
        new Date(anchor.getFullYear(), anchor.getMonth() + direction, 1),
      );
    else {
      const date = new Date(
        anchor.getFullYear(),
        anchor.getMonth(),
        anchor.getDate() + direction,
      );
      setAnchor(date);
      setSelected(date);
    }
  }
  function jump(year: number, month: number) {
    const date = new Date(
      year,
      month,
      Math.min(anchor.getDate(), new Date(year, month + 1, 0).getDate()),
    );
    setAnchor(date);
    setSelected(date);
    cancel();
  }
  function monthGrid(month: Date, compact = false) {
    const first = new Date(month.getFullYear(), month.getMonth(), 1);
    const gridStart = new Date(first);
    gridStart.setDate(1 - ((first.getDay() + 6) % 7));
    return (
      <div
        className={`rental-month-grid${compact ? " compact" : ""}`}
        aria-label={`${month.getFullYear()}年${month.getMonth() + 1}月日期`}
      >
        {["一", "二", "三", "四", "五", "六", "日"].map((day) => (
          <div className="rental-weekday" key={day}>
            周{day}
          </div>
        ))}
        {Array.from({ length: 42 }, (_, index) => {
          const date = new Date(
            gridStart.getFullYear(),
            gridStart.getMonth(),
            gridStart.getDate() + index,
          );
          const window = { start: date.getTime(), end: dayEnd(date).getTime() };
          const entries = rentalConflicts(
            bookings,
            window.start,
            window.end,
            excludeId,
          );
          const hours =
            available(window).reduce(
              (sum, item) => sum + item.end - item.start,
              0,
            ) / 3600000;
          const disabledCell =
            unavailable(window) ||
            (compact && date.getMonth() !== month.getMonth());
          return (
            <button
              type="button"
              key={date.toISOString()}
              className={`rental-day${unavailable(window) ? " unavailable" : ""}${date.getMonth() !== month.getMonth() ? " outside" : ""}${sameDay(date, today) ? " today" : ""}${selectedRange(window) ? " is-selected" : ""}`}
              data-calendar-start={window.start}
              data-calendar-end={window.end}
              aria-label={`${date.getFullYear()}年${date.getMonth() + 1}月${date.getDate()}日，${entries.length} 条租赁，空闲 ${Number(hours.toFixed(1))} 小时`}
              disabled={disabledCell}
              aria-pressed={
                selectedRange(window) || (!range && sameDay(date, selected))
              }
              onClick={(event) => {
                if (event.detail === 0) {
                  selectSingle(window);
                }
              }}
              onDoubleClick={() => {
                if (disabledCell) return;
                setAnchor(date);
                setSelected(date);
                setView("day");
              }}
            >
              <span className="calendar-date-number">
                {date.getDate()}
                {!compact && entries.length > 0 && (
                  <small>{entries.length} 条</small>
                )}
              </span>
              {!compact && (
                <div className="calendar-cell-events">
                  {entries.slice(0, 2).map((booking) => (
                    <span
                      key={booking.id}
                      className="calendar-booking"
                      title={booking.label || "已有租赁"}
                    >
                      {clock(
                        Math.max(window.start, Date.parse(booking.starts_at)),
                      )}{" "}
                      {booking.label || "已有租赁"}
                    </span>
                  ))}
                  {entries.length > 2 && (
                    <span>另 {entries.length - 2} 条租赁</span>
                  )}
                  {!entries.length && (
                    <span className="day-free">
                      {window.end <= cutoff ? "已结束" : "空闲"}
                    </span>
                  )}
                </div>
              )}
              {compact && entries.length > 0 && (
                <i className="calendar-busy-dot" />
              )}
            </button>
          );
        })}
      </div>
    );
  }
  const period = view === "year" ? "年" : view === "month" ? "月" : "天";
  return (
    <section
      ref={root}
      className="rental-calendar"
      aria-label="租赁日历"
      aria-busy={disabled}
      onPointerDown={pointerDown}
      onPointerMove={pointerMove}
      onPointerUp={pointerUp}
      onPointerCancel={cancel}
      onLostPointerCapture={cancel}
      onKeyDown={(event) => {
        if (event.key === "Escape") cancel();
      }}
    >
      <div className="rental-calendar-heading">
        <div className="calendar-navigation">
          <CalendarDays aria-hidden="true" />
          <Button
            type="button"
            variant="outline"
            size="icon"
            aria-label={`上一${period}`}
            onClick={() => navigate(-1)}
          >
            <ChevronLeft />
          </Button>
          <Button
            type="button"
            variant="outline"
            onClick={() => {
              setAnchor(dateOnly(today));
              setSelected(dateOnly(today));
            }}
          >
            今天
          </Button>
          <Button
            type="button"
            variant="outline"
            size="icon"
            aria-label={`下一${period}`}
            onClick={() => navigate(1)}
          >
            <ChevronRight />
          </Button>
          <NativeSelect
            aria-label="显示年份"
            value={anchor.getFullYear()}
            onChange={(event) =>
              jump(Number(event.target.value), anchor.getMonth())
            }
          >
            {Array.from(
              { length: 21 },
              (_, index) => anchor.getFullYear() - 10 + index,
            ).map((year) => (
              <option key={year} value={year}>
                {year} 年
              </option>
            ))}
          </NativeSelect>
          {view !== "year" && (
            <NativeSelect
              aria-label="显示月份"
              value={anchor.getMonth()}
              onChange={(event) =>
                jump(anchor.getFullYear(), Number(event.target.value))
              }
            >
              {Array.from({ length: 12 }, (_, month) => (
                <option key={month} value={month}>
                  {month + 1} 月
                </option>
              ))}
            </NativeSelect>
          )}
          {view === "day" && (
            <span className="calendar-current-day">{anchor.getDate()} 日</span>
          )}
        </div>
        <div
          className="calendar-view-switch"
          role="group"
          aria-label="日历视图"
        >
          {(
            [
              ["year", "年"],
              ["month", "月"],
              ["day", "日"],
            ] as const
          ).map(([mode, label]) => (
            <Button
              key={mode}
              type="button"
              variant={view === mode ? "default" : "outline"}
              aria-label={`${label}视图`}
              aria-pressed={view === mode}
              onClick={() => {
                cancel();
                setView(mode);
                if (mode === "day") setSelected(anchor);
              }}
            >
              {label}
            </Button>
          ))}
        </div>
      </div>
      <div className="calendar-status-line">
        <div className="rental-calendar-legend">
          <span>
            <i className="calendar-busy" />
            已有租赁
          </span>
          <span>
            <i className="calendar-free" />
            可用时段
          </span>
        </div>
        <p
          className={`calendar-selection-feedback${error ? " has-error" : ""}`}
          role={error ? "alert" : undefined}
          aria-live="polite"
        >
          {error ||
            (view === "day"
              ? "单击或拖拽选择时段，灰色不可选 · 浏览器时区"
              : "单击重新选择日期，拖拽多选，双击查看时段")}
        </p>
      </div>
      <div className="rental-calendar-body" ref={body}>
        {view === "month" && monthGrid(anchor)}
        {view === "year" && (
          <div className="calendar-year-grid">
            {Array.from({ length: 12 }, (_, month) => (
              <section className="calendar-mini-month" key={month}>
                <Button
                  type="button"
                  variant="ghost"
                  onClick={() => {
                    setAnchor(new Date(anchor.getFullYear(), month, 1));
                    setView("month");
                  }}
                >
                  {month + 1} 月
                </Button>
                {monthGrid(new Date(anchor.getFullYear(), month, 1), true)}
              </section>
            ))}
          </div>
        )}
        {view === "day" && (
          <div className="calendar-time-grid" aria-label="当日半小时时段">
            {slots.map((window) => {
              const entries = rentalConflicts(
                bookings,
                window.start,
                window.end,
                excludeId,
              );
              const label = `${clock(window.start, showSlotZone)}–${window.end === dayEnd(anchor).getTime() ? "24:00" : clock(window.end, showSlotZone)}`;
              const status =
                window.end <= cutoff
                  ? "已结束"
                  : entries.length
                    ? "已有租赁"
                    : "空闲";
              return (
                <button
                  key={window.start}
                  type="button"
                  className={`calendar-time-slot${unavailable(window) ? " unavailable" : ""}${entries.length ? " busy" : ""}${selectedRange(window) ? " is-selected" : ""}`}
                  data-calendar-start={window.start}
                  data-calendar-end={window.end}
                  aria-label={`${label}，${status}`}
                  disabled={unavailable(window)}
                  aria-pressed={selectedRange(window)}
                  onClick={(event) => {
                    if (event.detail === 0) selectSingle(window);
                  }}
                >
                  <time>{label}</time>
                  <span>
                    {status === "已结束"
                      ? status
                      : entries
                          .map((item) => item.label || "已有租赁")
                          .join("、") || "空闲"}
                  </span>
                </button>
              );
            })}
          </div>
        )}
      </div>
      <details className="rental-day-details">
        <summary>
          {selected.toLocaleDateString("zh-CN", {
            month: "long",
            day: "numeric",
            weekday: "long",
          })}{" "}
          · 当日可用时段 <ChevronDown aria-hidden="true" />
        </summary>
        <div className="calendar-details-content">
          <div>
            <h3>已有租赁</h3>
            {reservations.length ? (
              reservations.map((booking) => (
                <div className="calendar-reservation" key={booking.id}>
                  <span>
                    {clock(Math.max(start, Date.parse(booking.starts_at)))}–
                    {Math.min(end, Date.parse(booking.ends_at)) === end
                      ? "24:00"
                      : clock(Date.parse(booking.ends_at))}
                  </span>
                  <strong>{booking.label || "已租赁"}</strong>
                </div>
              ))
            ) : (
              <p>当日暂无租赁。</p>
            )}
          </div>
          <div>
            <h3>可用时段</h3>
            <div className="calendar-free-windows">
              {free.map((window) => (
                <Button
                  type="button"
                  variant="outline"
                  key={window.start}
                  disabled={window.end <= Math.ceil(Date.now() / 60000) * 60000}
                  onClick={() => select(window)}
                >
                  {clock(window.start)}–
                  {window.end === end ? "24:00" : clock(window.end)}
                </Button>
              ))}
              {!free.length && <p>当日已满，请选择其他日期或开发板。</p>}
            </div>
          </div>
        </div>
      </details>
    </section>
  );
}

import { useEffect, useState } from "react";
import { ChevronLeft, ChevronRight, CalendarDays } from "lucide-react";
import { Button } from "./ui/button";
import {
  freeWindows,
  rentalConflicts,
  type RentalWindow,
} from "@/utils/rental-calendar";

export function RentalCalendar({
  bookings,
  startsAt,
  excludeId,
  onSelect,
}: {
  bookings: RentalWindow[];
  startsAt: string;
  excludeId?: string;
  onSelect: (start: Date, end: Date) => void;
}) {
  const initial = new Date(startsAt);
  const valid = Number.isNaN(initial.getTime()) ? new Date() : initial;
  const [selected, setSelected] = useState(
    () => new Date(valid.getFullYear(), valid.getMonth(), valid.getDate()),
  );
  const [month, setMonth] = useState(
    () => new Date(valid.getFullYear(), valid.getMonth(), 1),
  );
  useEffect(() => {
    const date = new Date(startsAt);
    if (!Number.isNaN(date.getTime())) {
      setSelected(
        new Date(date.getFullYear(), date.getMonth(), date.getDate()),
      );
      setMonth(new Date(date.getFullYear(), date.getMonth(), 1));
    }
  }, [startsAt]);
  const first = new Date(month.getFullYear(), month.getMonth(), 1);
  const gridStart = new Date(first);
  gridStart.setDate(first.getDate() - ((first.getDay() + 6) % 7));
  const dayStart = selected.getTime(),
    dayEnd = new Date(
      selected.getFullYear(),
      selected.getMonth(),
      selected.getDate() + 1,
    ).getTime();
  const today = new Date();
  const reservations = rentalConflicts(
    bookings,
    dayStart,
    dayEnd,
    excludeId,
  ).sort((a, b) => Date.parse(a.starts_at) - Date.parse(b.starts_at));
  const free = freeWindows(bookings, dayStart, dayEnd, excludeId);
  const time = (value: number) =>
    value === dayEnd
      ? "24:00"
      : new Date(value).toLocaleTimeString("zh-CN", {
          hour: "2-digit",
          minute: "2-digit",
          hour12: false,
        });
  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate();
  return (
    <section className="rental-calendar" aria-label="租赁日历">
      <div className="rental-calendar-heading">
        <strong>
          <CalendarDays size={17} />
          {month.getFullYear()} 年 {month.getMonth() + 1} 月
        </strong>
        <div>
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            aria-label="上个月"
            onClick={() =>
              setMonth(new Date(month.getFullYear(), month.getMonth() - 1, 1))
            }
          >
            <ChevronLeft />
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => {
              setMonth(new Date(today.getFullYear(), today.getMonth(), 1));
              setSelected(
                new Date(
                  today.getFullYear(),
                  today.getMonth(),
                  today.getDate(),
                ),
              );
            }}
          >
            今天
          </Button>
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            aria-label="下个月"
            onClick={() =>
              setMonth(new Date(month.getFullYear(), month.getMonth() + 1, 1))
            }
          >
            <ChevronRight />
          </Button>
        </div>
      </div>
      <div className="rental-calendar-legend">
        <span>
          <i className="calendar-busy" />
          已有租赁
        </span>
        <span>
          <i className="calendar-free" />
          可用时段
        </span>
        <small>按浏览器时区显示；已取消租赁和待审核申请不占用时段</small>
      </div>
      <div className="rental-month-grid">
        {["一", "二", "三", "四", "五", "六", "日"].map((day) => (
          <div className="rental-weekday" key={day}>
            周{day}
          </div>
        ))}
        {Array.from({ length: 42 }, (_, index) => {
          const day = new Date(
              gridStart.getFullYear(),
              gridStart.getMonth(),
              gridStart.getDate() + index,
            ),
            end = new Date(
              day.getFullYear(),
              day.getMonth(),
              day.getDate() + 1,
            );
          const entries = rentalConflicts(
            bookings,
            day.getTime(),
            end.getTime(),
            excludeId,
          );
          const hours =
            freeWindows(
              bookings,
              day.getTime(),
              end.getTime(),
              excludeId,
            ).reduce((sum, window) => sum + window.end - window.start, 0) /
            3600000;
          return (
            <button
              type="button"
              key={day.toISOString()}
              aria-label={`${day.getFullYear()}年${day.getMonth() + 1}月${day.getDate()}日，${entries.length} 条租赁，空闲 ${Number(hours.toFixed(1))} 小时`}
              aria-pressed={sameDay(day, selected)}
              className={`rental-day${day.getMonth() !== month.getMonth() ? " outside" : ""}${sameDay(day, today) ? " today" : ""}`}
              onClick={() => setSelected(day)}
            >
              <span>{day.getDate()}</span>
              <small className={entries.length ? "day-busy" : "day-free"}>
                {entries.length ? `${entries.length} 条租赁` : "空闲"}
              </small>
              <em>
                {hours === 0 ? "已满" : `${Number(hours.toFixed(1))}h 可用`}
              </em>
            </button>
          );
        })}
      </div>
      <div className="rental-day-details">
        <div>
          <h3>
            {selected.toLocaleDateString("zh-CN", {
              month: "long",
              day: "numeric",
              weekday: "long",
            })}{" "}
            · 已有租赁
          </h3>
          {reservations.length ? (
            reservations.map((booking) => (
              <div className="calendar-reservation" key={booking.id}>
                <span>
                  {time(Math.max(dayStart, Date.parse(booking.starts_at)))}–
                  {time(Math.min(dayEnd, Date.parse(booking.ends_at)))}
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
          <p>点击填入起止时间，保存前可继续调整。</p>
          <div className="calendar-free-windows">
            {free.map((window) => (
              <Button
                type="button"
                size="sm"
                variant="outline"
                key={window.start}
                disabled={window.end <= Math.ceil(Date.now() / 60000) * 60000}
                onClick={() =>
                  onSelect(
                    new Date(
                      Math.max(
                        window.start,
                        Math.ceil(Date.now() / 60000) * 60000,
                      ),
                    ),
                    new Date(window.end),
                  )
                }
              >
                {time(window.start)}–{time(window.end)}
              </Button>
            ))}
            {!free.length && <p>当日已满，请选择其他日期或开发板。</p>}
          </div>
        </div>
      </div>
    </section>
  );
}

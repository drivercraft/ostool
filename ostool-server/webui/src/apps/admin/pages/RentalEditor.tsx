import { useId, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { PagePanel } from "@/components/page-panel";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Notice, TextField, SelectField, useAction } from "@/components/forms";
import { RentalCalendar } from "@/components/rental-calendar";
import { api } from "@/api/client";
import { useResource } from "@/api/events";
import { localDateTime, rentalConflicts } from "@/utils/rental-calendar";
import type {
  BoardConfig,
  ManagedUser,
  Rental,
  RentalInput,
} from "@/types/api";

export default function RentalEditor() {
  const { rentalId } = useParams();
  const users = useResource("users"),
    boards = useResource("boards"),
    rentals = useResource("rentals");
  if (!users || !boards || !rentals)
    return (
      <PagePanel>
        <Skeleton className="h-48 w-full" />
      </PagePanel>
    );
  const rental = rentals.find((r) => r.id === rentalId);
  if (rentalId && !rental)
    return (
      <PagePanel
        footer={
          <Button asChild variant="outline">
            <Link to="/rentals">返回租赁列表</Link>
          </Button>
        }
      >
        <Notice>租赁不存在或已删除。</Notice>
      </PagePanel>
    );
  return (
    <RentalForm
      key={rentalId ?? "new"}
      initial={rental}
      users={users}
      boards={boards}
      rentals={rentals}
    />
  );
}
function RentalForm({
  initial,
  users,
  boards,
  rentals,
}: {
  initial?: Rental;
  users: ManagedUser[];
  boards: BoardConfig[];
  rentals: Rental[];
}) {
  const [draft, setDraft] = useState<RentalInput>(() => ({
    board_id: initial?.board_id ?? boards.find((b) => !b.disabled)?.id ?? "",
    user_id: initial?.user_id ?? users.find((u) => !u.disabled)?.id ?? "",
    starts_at: localDateTime(new Date(initial?.starts_at ?? Date.now())),
    ends_at: localDateTime(new Date(initial?.ends_at ?? Date.now() + 3600000)),
    notes: initial?.notes ?? "",
  }));
  const original = useRef(JSON.stringify(initial));
  const current = rentals.find((r) => r.id === initial?.id);
  const task = useAction(),
    navigate = useNavigate(),
    formId = useId();
  const bookings = rentals
    .filter((r) => r.board_id === draft.board_id)
    .map((r) => ({
      ...r,
      label:
        users.find((u) => u.id === r.user_id)?.display_name ||
        users.find((u) => u.id === r.user_id)?.email ||
        "已有租赁",
    }));
  const conflicts = rentalConflicts(
    bookings,
    Date.parse(draft.starts_at),
    Date.parse(draft.ends_at),
    initial?.id,
  );
  const finished =
    !!initial &&
    (current?.status === "cancelled" ||
      current?.status === "expired" ||
      !current);
  const patch = (change: Partial<RentalInput>) =>
    setDraft((value) => ({ ...value, ...change }));
  const back = (
    <Button asChild variant="outline">
      <Link to="/rentals">取消</Link>
    </Button>
  );
  return (
    <PagePanel
      className="rental-editor-panel"
      toolbar={
        <form
          id={formId}
          className="rental-editor-form"
          onSubmit={(event) => {
            event.preventDefault();
            void task
              .run(async () => {
                const start = new Date(draft.starts_at),
                  end = new Date(draft.ends_at);
                if (
                  !Number.isFinite(start.getTime()) ||
                  !Number.isFinite(end.getTime()) ||
                  start >= end
                )
                  throw new Error("开始时间必须早于结束时间");
                if (
                  rentalConflicts(
                    bookings,
                    start.getTime(),
                    end.getTime(),
                    initial?.id,
                  ).length
                )
                  throw new Error("该时段已被租用，请调整时间");
                const payload = {
                  ...draft,
                  starts_at: start.toISOString(),
                  ends_at: end.toISOString(),
                };
                if (initial) await api.updateRental(initial.id, payload);
                else await api.createRental(payload);
              }, "租赁已保存")
              .then((ok) => {
                if (ok) navigate("/rentals");
              });
          }}
        >
          <SelectField
            label="使用用户"
            disabled={!!initial || task.pending}
            value={draft.user_id}
            onValue={(user_id) => patch({ user_id })}
            options={users
              .filter((u) => !u.disabled || u.id === draft.user_id)
              .map((u) => ({
                value: u.id,
                label: `${u.display_name || u.email} · ${u.email}`,
              }))}
          />
          <SelectField
            label="指定开发板"
            disabled={!!initial || task.pending}
            value={draft.board_id}
            onValue={(board_id) => patch({ board_id })}
            options={boards
              .filter((b) => !b.disabled || b.id === draft.board_id)
              .map((b) => ({
                value: b.id,
                label: `${b.id} · ${b.board_type}`,
              }))}
          />
          <TextField
            label="开始时间"
            type="datetime-local"
            step="0.001"
            required
            value={draft.starts_at}
            onValue={(starts_at) => patch({ starts_at })}
          />
          <TextField
            label="结束时间"
            type="datetime-local"
            step="0.001"
            required
            value={draft.ends_at}
            onValue={(ends_at) => patch({ ends_at })}
          />
          <TextField
            label="备注"
            value={draft.notes}
            onValue={(notes) => patch({ notes })}
          />
        </form>
      }
      footer={
        <div className="editor-footer">
          <span className="panel-status">
            {conflicts.length
              ? `所选时段有 ${conflicts.length} 条租赁冲突`
              : "日历与已有租赁实时同步"}
          </span>
          <div className="row-actions">
            {back}
            <Button
              type="submit"
              form={formId}
              disabled={
                task.pending ||
                finished ||
                !draft.board_id ||
                !draft.user_id ||
                conflicts.length > 0
              }
            >
              {task.pending ? "保存中…" : "保存租赁"}
            </Button>
          </div>
        </div>
      }
    >
      {task.error && <Notice>{task.error}</Notice>}
      {initial && JSON.stringify(current) !== original.current && (
        <Notice>
          该租赁状态已更新，当前编辑内容已保留，请结合最新日历确认。
        </Notice>
      )}
      {conflicts.length > 0 && (
        <Notice>
          所选时段已有租赁。请在日历中选择空闲时段，或修改起止时间。
        </Notice>
      )}

      <RentalCalendar
        bookings={bookings}
        startsAt={draft.starts_at}
        endsAt={draft.ends_at}
        excludeId={initial?.id}
        onSelect={(start, end) =>
          patch({
            starts_at: localDateTime(start),
            ends_at: localDateTime(end),
          })
        }
      />
    </PagePanel>
  );
}

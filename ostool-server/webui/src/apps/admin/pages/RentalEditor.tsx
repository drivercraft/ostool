import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { PagePanel } from "@/components/page-panel";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Notice, TextField, SelectField, useAction } from "@/components/forms";
import { RentalCalendar } from "@/components/rental-calendar";
import { api } from "@/api/client";
import { admin, useErrors, useResource } from "@/api/events";
import { queryKey } from "@/components/server-list";
import { localDateTime, rentalConflicts } from "@/utils/rental-calendar";
import type {
  AdminBoardSummary,
  RentalEditorData,
  Rental,
  RentalInput,
} from "@/types/api";

export default function RentalEditor() {
  const { rentalId } = useParams();
  const boards = useResource("boards"),
    context = useResource("rental_editor"),
    error = useErrors().rental_editor;
  useEffect(() => {
    if (!context || (context.query.id ?? undefined) !== rentalId)
      admin.setEditorFilters({ id: rentalId });
    return () => admin.setEditorFilters(null);
  }, [rentalId]);
  if (error && !context)
    return (
      <PagePanel>
        <Notice>{error}</Notice>
      </PagePanel>
    );
  if (!context || (context.query.id ?? undefined) !== rentalId || !boards)
    return (
      <PagePanel>
        <Skeleton className="h-48 w-full" />
      </PagePanel>
    );
  const rental = context.rental ?? undefined;
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
      boards={boards}
      context={context}
    />
  );
}
function RentalForm({
  initial,
  boards,
  context,
}: {
  initial?: Rental;
  boards: AdminBoardSummary[];
  context: RentalEditorData;
}) {
  const users = context.users;
  const [draft, setDraft] = useState<RentalInput>(() => ({
    board_id: initial?.board_id ?? boards.find((b) => !b.disabled)?.id ?? "",
    user_id: initial?.user_id ?? users.find((u) => !u.disabled)?.id ?? "",
    starts_at: localDateTime(new Date(initial?.starts_at ?? Date.now())),
    ends_at: localDateTime(new Date(initial?.ends_at ?? Date.now() + 3600000)),
    notes: initial?.notes ?? "",
  }));
  const original = useRef(JSON.stringify(initial));
  const current = context.rental;
  const [userSearch, setUserSearch] = useState("");
  const [range, setRange] = useState<{ from?: string; to?: string }>({});
  const onRangeChange = useCallback(
    (from: Date, to: Date) =>
      setRange((old) =>
        old.from === from.toISOString() && old.to === to.toISOString()
          ? old
          : { from: from.toISOString(), to: to.toISOString() },
      ),
    [],
  );
  const filters = {
    id: initial?.id,
    user: draft.user_id || undefined,
    q: userSearch,
    board: draft.board_id,
    ...range,
  };
  const identity = queryKey(filters);
  const previousSearch = useRef(userSearch);
  useEffect(() => {
    const delayed = previousSearch.current !== userSearch;
    previousSearch.current = userSearch;
    if (!delayed) {
      admin.setEditorFilters(filters);
      return;
    }
    const timer = setTimeout(() => admin.setEditorFilters(filters), 300);
    return () => clearTimeout(timer);
  }, [identity]);
  const ready = queryKey(context.query) === identity;
  const error = useErrors().rental_editor;
  const task = useAction(),
    navigate = useNavigate(),
    formId = useId();
  const bookings = context.bookings;
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
                if (!ready || error) throw new Error("请等待日历同步完成");
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
                label: `${u.username} · ${u.email}${u.display_name ? ` · ${u.display_name}` : ""}`,
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
          {(context.more_users || userSearch) && (
            <div className="rental-user-search">
              <TextField
                label="搜索使用用户"
                value={userSearch}
                onValue={setUserSearch}
                placeholder={
                  context.more_users
                    ? "输入用户名或邮箱缩小范围（显示前 50 项）"
                    : "按用户名、邮箱或显示名称搜索"
                }
              />
            </div>
          )}
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
            {!ready
              ? "正在同步日历…"
              : conflicts.length
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
                !ready ||
                !!error ||
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
      {error && <Notice>{error}</Notice>}
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
        disabled={!ready || !!error}
        onRangeChange={onRangeChange}
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

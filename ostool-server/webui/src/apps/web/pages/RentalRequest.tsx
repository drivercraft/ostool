import { useCallback, useEffect, useId, useState } from "react";
import { Link, Navigate, useLocation, useNavigate } from "react-router-dom";
import { api, type AuthUser } from "@/api/client";
import {
  account,
  useAccountConnection,
  useAccountErrors,
  useAccountResource,
  site,
  useDeviceDocument,
} from "@/api/events";
import { useSiteSession } from "@/apps/web/session";
import { BoardVisual } from "@/apps/web/components/board-visual";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  TextField,
  SelectField,
  Notice,
  FormGrid,
  useAction,
} from "@/components/forms";
import { PagePanel, EditorFooter } from "@/components/page-panel";
import { queryKey } from "@/components/server-list";
import type { CatalogBoard } from "@/types/api";
import { RentalCalendar } from "@/components/rental-calendar";
import {
  localDateTime,
  rentalConflicts,
  type RentalWindow,
} from "@/utils/rental-calendar";

const localDate = (offset: number) =>
  localDateTime(new Date(Date.now() + offset));

/** A single application page shared by device cards and the member workspace. */
export default function RentalRequest() {
  const { user, checking, error } = useSiteSession();
  const location = useLocation();
  const connection = useAccountConnection();
  if (!checking && !user && !error)
    return (
      <Navigate
        replace
        to={`/login?redirect=${encodeURIComponent(location.pathname + location.search)}`}
      />
    );
  return (
    <main className="public-inner-page rental-request-page">
      <div className="member-heading">
        <div>
          <h1>租赁申请</h1>
          <p>选择开发板及使用时段，提交后等待管理员审核。</p>
        </div>
        <Button asChild variant="outline">
          <Link to="/account/requests">申请记录</Link>
        </Button>
      </div>
      {checking ? (
        <Skeleton className="h-72 w-full" />
      ) : error ? (
        <Notice>{error}</Notice>
      ) : (
        user && (
          <>
            {connection === "reconnecting" && (
              <Notice>连接暂时中断，输入已保留，恢复后自动同步。</Notice>
            )}
            <RentalRequestEditor user={user} />
          </>
        )
      )}
    </main>
  );
}

function RentalRequestEditor({ user }: { user: AuthUser }) {
  const catalog = useAccountResource("catalog"),
    availability = useAccountResource("availability") ?? [];
  if (!catalog)
    return (
      <PagePanel>
        <Skeleton className="h-48 w-full" />
      </PagePanel>
    );
  if (!catalog.length)
    return (
      <PagePanel
        className="rental-request-panel"
        footer={
          <Button asChild variant="outline">
            <Link to="/account/requests">取消</Link>
          </Button>
        }
      >
        <Notice>暂无可申请的设备，请联系管理员。</Notice>
      </PagePanel>
    );
  return (
    <RentalRequestForm
      user={user}
      catalog={catalog}
      availability={availability}
    />
  );
}
function RentalRequestForm({
  user,
  catalog,
  availability,
}: {
  user: AuthUser;
  catalog: CatalogBoard[];
  availability: RentalWindow[];
}) {
  const selectedDocument = useDeviceDocument();
  const location = useLocation(),
    navigate = useNavigate(),
    formId = useId(),
    task = useAction();
  const [board, setBoard] = useState(() => {
    const requested = new URLSearchParams(location.search).get("board");
    return requested ?? catalog[0]?.id ?? "";
  });
  const [start, setStart] = useState(() => localDate(3600000)),
    [end, setEnd] = useState(() => localDate(7200000)),
    [notes, setNotes] = useState("");
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
  const filters = { board, ...range },
    identity = queryKey(filters);
  useEffect(() => {
    account.setEditorFilters(filters);
  }, [identity]);
  useEffect(() => () => account.setEditorFilters(null), []);
  const query = useAccountResource("availability_query"),
    error = useAccountErrors().availability;
  const ready = !!query && queryKey(query) === identity && !error;
  const bookings = availability.filter((item) => item.board_id === board);
  const conflicts = rentalConflicts(
    bookings,
    Date.parse(start),
    Date.parse(end),
  );
  const valid =
    Number.isFinite(Date.parse(start)) &&
    Number.isFinite(Date.parse(end)) &&
    Date.parse(start) < Date.parse(end);
  const selectedBoard = catalog.find((item) => item.id === board);
  const boardExists = !!selectedBoard;
  const document =
    selectedDocument?.id === board ? selectedDocument.document : null;
  useEffect(() => {
    site.setDeviceDocument(board || null);
    return () => site.setDeviceDocument(null);
  }, [board]);
  return (
    <div className="rental-request-layout">
      <aside className="rental-device-summary" aria-label="所选设备信息">
        <div className="rental-device-image">
          {document?.image_url ? (
            <img
              src={document.image_url}
              alt={`${document.title || board} 开发板`}
              referrerPolicy="no-referrer"
            />
          ) : (
            <BoardVisual
              label={selectedBoard?.board_type || "DEVELOPMENT BOARD"}
              compact
            />
          )}
        </div>
        <div className="rental-device-copy">
          <span className="section-label">YOUR NEXT HARDWARE</span>
          <h2>{document?.title || board || "选择开发板"}</h2>
          <p>
            {document?.summary || "选择使用时段，给下一次开发留出专属空间。"}
          </p>
          <div className="device-detail-tags">
            {selectedBoard?.tags.map((tag) => (
              <span key={tag}>{tag}</span>
            ))}
          </div>
          <dl>
            <div>
              <dt>设备标识</dt>
              <dd>{board || "—"}</dd>
            </div>
            <div>
              <dt>开发板类型</dt>
              <dd>{selectedBoard?.board_type || "—"}</dd>
            </div>
            {document?.specifications.map((item, index) => (
              <div key={index}>
                <dt>{item.name}</dt>
                <dd>{item.value}</dd>
              </div>
            ))}
          </dl>
          {boardExists && (
            <Button asChild variant="outline">
              <Link to={`/devices/${encodeURIComponent(board)}`}>
                查看设备详情
              </Link>
            </Button>
          )}
        </div>
      </aside>
      <PagePanel
        className="rental-editor-panel rental-request-panel"
        toolbar={
          <FormGrid asChild columns={2}>
            <form
              id={formId}
              className="rental-editor-form"
              onSubmit={(event) => {
                event.preventDefault();
                void task
                  .run(async () => {
                    if (!boardExists) throw new Error("请选择可申请的设备");
                    if (!valid) throw new Error("开始时间必须早于结束时间");
                    if (conflicts.length)
                      throw new Error("所选时段已有租赁，请选择空闲时段");
                    if (!ready) throw new Error("请等待日历同步完成");
                    await api.applyRental({
                      board_id: board,
                      starts_at: new Date(start).toISOString(),
                      ends_at: new Date(end).toISOString(),
                      notes,
                    });
                  }, "申请已提交，等待管理员审核")
                  .then((ok) => {
                    if (ok) navigate("/account/requests");
                  });
              }}
            >
              <TextField
                label="申请用户"
                value={
                  user.display_name
                    ? user.display_name + " · " + user.email
                    : user.email
                }
                readOnly
              />
              <SelectField
                label="申请开发板"
                value={board}
                onValue={setBoard}
                disabled={task.pending}
                options={catalog.map((item) => ({
                  value: item.id,
                  label: item.id + " · " + item.board_type,
                }))}
              />
              <TextField
                label="开始时间"
                type="datetime-local"
                step="0.001"
                required
                value={start}
                onValue={setStart}
              />
              <TextField
                label="结束时间"
                type="datetime-local"
                step="0.001"
                required
                value={end}
                onValue={setEnd}
              />
              <TextField
                label="申请说明"
                fullWidth
                maxLength={2000}
                value={notes}
                onValue={setNotes}
              />
            </form>
          </FormGrid>
        }
        footer={
          <EditorFooter
            status={
              conflicts.length
                ? "所选时段已有租赁"
                : "申请不预占设备，通过审核后生效"
            }
          >
            <Button asChild variant="outline">
              <Link to="/account/requests">取消</Link>
            </Button>
            <Button
              type="submit"
              form={formId}
              disabled={
                task.pending ||
                !ready ||
                !boardExists ||
                !valid ||
                conflicts.length > 0
              }
            >
              {task.pending ? "提交中…" : "提交申请"}
            </Button>
          </EditorFooter>
        }
      >
        {error && <Notice>{error}</Notice>}
        {task.error && <Notice>{task.error}</Notice>}
        {!boardExists && <Notice>所选设备目前不可申请，请重新选择。</Notice>}
        {conflicts.length > 0 && (
          <Notice>所选时段已有租赁，请调整时间或在日历中选择空闲时段。</Notice>
        )}
        {boardExists && (
          <RentalCalendar
            disabled={!ready}
            onRangeChange={onRangeChange}
            bookings={bookings}
            startsAt={start}
            endsAt={end}
            onSelect={(from, to) => {
              setStart(localDateTime(from));
              setEnd(localDateTime(to));
            }}
          />
        )}
      </PagePanel>
    </div>
  );
}

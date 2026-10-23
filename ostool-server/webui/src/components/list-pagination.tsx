import { useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { NativeSelect } from "@/components/ui/native-select";

export function ListPagination({
  total,
  page,
  size,
  pending = false,
  onPage,
  onSize,
}: {
  total: number;
  page: number;
  size: number;
  pending?: boolean;
  onPage: (page: number) => void;
  onSize: (size: number) => void;
}) {
  const pages = Math.max(1, Math.ceil(total / size));
  return (
    <nav className="list-pagination" aria-label="分页">
      <span className="pagination-summary">
        {pending
          ? "正在同步…"
          : `共 ${total} 条 · ${total ? (page - 1) * size + 1 : 0}–${Math.min(page * size, total)} 条`}
      </span>
      <div className="pagination-actions">
        <NativeSelect
          aria-label="每页条数"
          value={size}
          onChange={(e) => onSize(Number(e.target.value))}
        >
          {[10, 20, 50, 100].map((n) => (
            <option key={n} value={n}>
              {n} 条/页
            </option>
          ))}
        </NativeSelect>
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          aria-label="上一页"
          disabled={pending || page <= 1}
          onClick={() => onPage(page - 1)}
        >
          <ChevronLeft />
        </Button>
        <span className="pagination-position">
          第 {page} / {pages} 页
        </span>
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          aria-label="下一页"
          disabled={pending || page >= pages}
          onClick={() => onPage(page + 1)}
        >
          <ChevronRight />
        </Button>
      </div>
    </nav>
  );
}

/** Pagination is local to the SSE projection. Updates do not reset filters/page. */
export function useListPage<T>(items: T[], filterKey: string) {
  const [selection, setSelection] = useState({ filterKey, page: 1, size: 20 });
  const total = items.length;
  const pages = Math.max(1, Math.ceil(total / selection.size));
  const page =
    selection.filterKey === filterKey ? Math.min(selection.page, pages) : 1;
  // Clamp once on removal so a later push does not unexpectedly jump forward.
  if (selection.filterKey !== filterKey || selection.page !== page)
    setSelection({ ...selection, filterKey, page });
  const start = (page - 1) * selection.size;
  return {
    viewKey: JSON.stringify([filterKey, page, selection.size]),
    rows: items.slice(start, start + selection.size),
    footer: (
      <ListPagination
        total={total}
        page={page}
        size={selection.size}
        onPage={(page) => setSelection({ ...selection, page })}
        onSize={(size) => setSelection({ filterKey, page: 1, size })}
      />
    ),
  };
}

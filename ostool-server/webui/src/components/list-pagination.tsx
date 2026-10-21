import { useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { NativeSelect } from "@/components/ui/native-select";

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
      <nav className="list-pagination" aria-label="分页">
        <span className="pagination-summary">
          共 {total} 条 ·{" "}
          {total
            ? `${start + 1}–${Math.min(start + selection.size, total)}`
            : "0"}{" "}
          条
        </span>
        <div className="pagination-actions">
          <NativeSelect
            aria-label="每页条数"
            value={selection.size}
            onChange={(e) =>
              setSelection({ filterKey, page: 1, size: Number(e.target.value) })
            }
          >
            {[10, 20, 50, 100].map((size) => (
              <option key={size} value={size}>
                {size} 条/页
              </option>
            ))}
          </NativeSelect>
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-label="上一页"
            disabled={page === 1}
            onClick={() => setSelection({ ...selection, page: page - 1 })}
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
            disabled={page === pages}
            onClick={() => setSelection({ ...selection, page: page + 1 })}
          >
            <ChevronRight />
          </Button>
        </div>
      </nav>
    ),
  };
}

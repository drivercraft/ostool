import type { ReactNode } from "react";
export function ListToolbar({
  actions,
  children,
  count,
}: {
  actions?: ReactNode;
  children?: ReactNode;
  count?: string;
}) {
  return (
    <div className="listing-toolbar" role="toolbar" aria-label="列表快捷工具栏">
      <div className="listing-actions">
        {actions}
        {count && <span className="listing-meta">{count}</span>}
      </div>
      <div className="listing-filters">{children}</div>
    </div>
  );
}

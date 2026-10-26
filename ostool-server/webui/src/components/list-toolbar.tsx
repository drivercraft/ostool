import { useContext, type ReactNode } from "react";
import { ListDataLoadingContext } from "./forms";
import { ListLayoutToggle } from "./list-layout";
export function ListToolbar({
  actions,
  children,
  count,
}: {
  actions?: ReactNode;
  children?: ReactNode;
  count?: string;
}) {
  const loading = useContext(ListDataLoadingContext);
  return (
    <div className="listing-toolbar" role="toolbar" aria-label="列表快捷工具栏">
      <div className="listing-actions">
        {actions}
        {count && (
          <span className="listing-meta">{loading ? "正在加载…" : count}</span>
        )}
      </div>
      <div className="listing-filters">
        {children}
        <ListLayoutToggle />
      </div>
    </div>
  );
}

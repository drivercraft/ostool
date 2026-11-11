import { useContext, type ComponentProps, type ReactNode } from "react";
import { ListDataLoadingContext } from "./forms";
import { ListLayoutToggle } from "./list-layout";
export function ListToolbar({
  actions,
  children,
  count,
  className = "",
  ...props
}: ComponentProps<"div"> & {
  actions?: ReactNode;
  count?: string;
}) {
  const loading = useContext(ListDataLoadingContext);
  return (
    <div
      {...props}
      className={`listing-toolbar ${className}`}
      role="toolbar"
      aria-label={props["aria-label"] ?? "列表快捷工具栏"}
    >
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

import { Children, cloneElement, isValidElement, type ReactNode } from "react";
import { SelectField, TextField } from "./forms";
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
  return (
    <div className="listing-toolbar" role="toolbar" aria-label="列表快捷工具栏">
      <div className="listing-actions">
        {actions}
        {count && <span className="listing-meta">{count}</span>}
      </div>
      <div className="listing-filters">
        {Children.map(children, (child) =>
          isValidElement<{ inlineHint?: boolean; hint?: string }>(child)
            ? child.type === TextField
              ? cloneElement(child, { inlineHint: true })
              : child.type === SelectField
                ? cloneElement(child, { hint: "" })
                : child
            : child,
        )}
        <ListLayoutToggle />
      </div>
    </div>
  );
}

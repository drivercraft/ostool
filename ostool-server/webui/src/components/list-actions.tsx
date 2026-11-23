import { Children, cloneElement, isValidElement, type ReactNode } from "react";
import { MoreActions } from "./more-actions";

/** Show common actions directly, with at most three buttons including the menu. */
export function ListActions({
  primary,
  edit,
  enable,
  children,
}: {
  primary?: ReactNode[];
  edit?: ReactNode;
  enable?: ReactNode;
  children?: ReactNode;
}) {
  const actions = Children.toArray(primary ?? [edit, enable]).map(
    (action, index) =>
      isValidElement(action)
        ? cloneElement(action, { key: action.key ?? index })
        : action,
  );
  const menu = Children.toArray(children).map((action, index) =>
    isValidElement(action)
      ? cloneElement(action, { key: action.key ?? index })
      : action,
  );
  const hasMore = menu.length > 0 || actions.length > 3;
  const count = hasMore ? 2 : 3;
  return (
    <div className="actions">
      {actions.slice(0, count)}
      {hasMore && (
        <MoreActions>
          {actions.slice(count)}
          {menu}
        </MoreActions>
      )}
    </div>
  );
}

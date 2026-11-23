import { useContext, type MouseEvent, type ComponentProps } from "react";
import { RowMenuContext } from "./more-actions";
import { DropdownMenuItem } from "./ui/dropdown-menu";
import { usePermissions } from "./permissions";
import type { LucideIcon } from "lucide-react";
import { Link } from "react-router-dom";
import { Button } from "./ui/button";

/** Consistent, named icon actions for table rows and their card equivalents. */
export function RowAction({
  label,
  icon: Icon,
  to,
  keepMenuOpen = false,
  variant = "ghost",
  ...props
}: Omit<ComponentProps<typeof Button>, "children" | "size" | "asChild"> & {
  label: string;
  icon: LucideIcon;
  to?: string;
  keepMenuOpen?: boolean;
}) {
  const menu = useContext(RowMenuContext);
  const { can } = usePermissions();
  if (props.permission && !can(props.permission)) return null;
  if (menu) {
    const content = (
      <>
        <Icon aria-hidden="true" />
        {label}
      </>
    );
    return (
      <DropdownMenuItem
        disabled={props.disabled}
        className={
          variant === "destructive"
            ? "text-destructive focus:text-destructive"
            : undefined
        }
        asChild={!!to && !props.disabled}
        onSelect={(event) => {
          if (!to) {
            // Confirmation dialogs remain mounted until dismissed.
            if (keepMenuOpen) event.preventDefault();
            props.onClick?.(event as unknown as MouseEvent<HTMLButtonElement>);
          }
        }}
      >
        {to && !props.disabled ? <Link to={to}>{content}</Link> : content}
      </DropdownMenuItem>
    );
  }
  return (
    <Button
      type="button"
      className="row-action"
      variant={variant}
      size="icon"
      aria-label={label}
      title={label}
      asChild={!!to && !props.disabled}
      {...props}
    >
      {to && !props.disabled ? (
        <Link to={to}>
          <Icon aria-hidden="true" />
        </Link>
      ) : (
        <Icon aria-hidden="true" />
      )}
    </Button>
  );
}

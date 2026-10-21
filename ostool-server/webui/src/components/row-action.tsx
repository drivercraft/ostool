import type { ComponentProps } from "react";
import type { LucideIcon } from "lucide-react";
import { Link } from "react-router-dom";
import { Button } from "./ui/button";

/** Consistent, named icon actions for table rows and their card equivalents. */
export function RowAction({
  label,
  icon: Icon,
  to,
  variant = "ghost",
  ...props
}: Omit<ComponentProps<typeof Button>, "children" | "size" | "asChild"> & {
  label: string;
  icon: LucideIcon;
  to?: string;
}) {
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

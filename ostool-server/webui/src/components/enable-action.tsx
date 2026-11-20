import { Power, PowerOff } from "lucide-react";
import { RowAction } from "./row-action";
import { useAction } from "./forms";

/** The target state is explicit; live rows continue to come from SSE. */
export function EnableAction({
  enabled,
  disabled,
  permission,
  action,
  name,
}: {
  enabled: boolean;
  disabled?: boolean;
  permission: string;
  action: (enabled: boolean) => Promise<unknown>;
  name: string;
}) {
  const task = useAction();
  const label = enabled ? "禁用" : "启用";
  return (
    <RowAction
      label={label}
      icon={enabled ? PowerOff : Power}
      permission={permission}
      disabled={disabled || task.pending}
      onClick={() => void task.run(() => action(!enabled), `${name}已${label}`)}
    />
  );
}

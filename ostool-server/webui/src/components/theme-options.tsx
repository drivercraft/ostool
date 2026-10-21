import { Sun, Moon, Monitor } from "lucide-react";
import { useTheme } from "next-themes";
import {
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
} from "./ui/dropdown-menu";
/** Shared by public and management menus, including portal-rendered styles. */
export function ThemeOptions() {
  const { theme, setTheme } = useTheme();
  return (
    <>
      <DropdownMenuLabel>主题</DropdownMenuLabel>
      <DropdownMenuSeparator />
      <DropdownMenuRadioGroup
        className="theme-menu-options"
        value={theme ?? "system"}
        onValueChange={setTheme}
      >
        {(
          [
            ["light", "明亮", Sun],
            ["dark", "黑暗", Moon],
            ["system", "跟随系统", Monitor],
          ] as const
        ).map(([value, label, Icon]) => (
          <DropdownMenuRadioItem key={value as string} value={value as string}>
            <Icon className="theme-option-icon" width={16} height={16} />
            {label as string}
          </DropdownMenuRadioItem>
        ))}
      </DropdownMenuRadioGroup>
    </>
  );
}

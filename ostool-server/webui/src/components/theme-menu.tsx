import { Monitor, Moon, Sun } from "lucide-react";
import { useTheme } from "next-themes";
import { Button } from "./ui/button";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
} from "./ui/dropdown-menu";
export function ThemeMenu({ label = "切换网站主题" }: { label?: string } = {}) {
  const { theme, resolvedTheme } = useTheme();
  const Icon =
    theme === "system" ? Monitor : resolvedTheme === "dark" ? Moon : Sun;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          size="icon"
          variant="ghost"
          className="site-theme-button"
          aria-label={label}
        >
          <Icon size={18} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="topbar-dropdown theme-menu-panel"
      >
        <ThemeOptions />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

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

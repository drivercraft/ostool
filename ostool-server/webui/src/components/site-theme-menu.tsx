import { ThemeOptions } from "./theme-options";
import { Monitor, Moon, Sun } from "lucide-react";
import { useTheme } from "next-themes";
import { Button } from "./ui/button";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
} from "./ui/dropdown-menu";
export function SiteThemeMenu() {
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
          aria-label="切换网站主题"
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

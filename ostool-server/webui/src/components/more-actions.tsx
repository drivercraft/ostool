import { createContext, type ReactNode } from "react";
import { Ellipsis } from "lucide-react";
import { Button } from "./ui/button";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
} from "./ui/dropdown-menu";

export const RowMenuContext = createContext(false);

export function MoreActions({ children }: { children: ReactNode }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="row-action"
          aria-label="更多"
          title="更多"
        >
          <Ellipsis aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <RowMenuContext.Provider value>{children}</RowMenuContext.Provider>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

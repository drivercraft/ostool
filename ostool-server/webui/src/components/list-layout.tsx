import { createContext, useContext, useState, type ReactNode } from "react";
import { LayoutGrid, List } from "lucide-react";
import { Button } from "./ui/button";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
} from "./ui/dropdown-menu";
type Layout = "list" | "cards";
const Context = createContext<{
  layout: Layout;
  setLayout: (layout: Layout) => void;
} | null>(null);
export function ListLayoutProvider({
  children,
  storageKey,
}: {
  children: ReactNode;
  storageKey: string;
}) {
  const key = `ostool.list-layout:${storageKey}`;
  const [layout, setValue] = useState<Layout>(() => {
    try {
      return localStorage.getItem(key) === "cards" ? "cards" : "list";
    } catch {
      return "list";
    }
  });
  function setLayout(value: Layout) {
    setValue(value);
    try {
      localStorage.setItem(key, value);
    } catch {
      /* Browser storage can be unavailable. */
    }
  }
  return (
    <Context.Provider value={{ layout, setLayout }}>
      {children}
    </Context.Provider>
  );
}
export const useListLayout = () => useContext(Context);
export function ListLayoutToggle() {
  const view = useListLayout();
  if (!view) return null;
  const Icon = view.layout === "cards" ? LayoutGrid : List;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label="切换列表布局"
          title="切换列表布局"
        >
          <Icon size={16} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuRadioGroup
          value={view.layout}
          onValueChange={(value) => view.setLayout(value as Layout)}
        >
          <DropdownMenuRadioItem value="list">
            <List />
            列表视图
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="cards">
            <LayoutGrid />
            卡片视图
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

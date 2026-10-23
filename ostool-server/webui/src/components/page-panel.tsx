import { useContext, useLayoutEffect, useRef, type ReactNode } from "react";
import { ListLayoutProvider } from "./list-layout";
import { ListDataLoadingContext } from "./forms";

/** Fixed header/footer with a single independently scrolling content region. */
export function PagePanel({
  toolbar,
  footer,
  children,
  table = false,
  scrollKey,
  layoutKey,
  className = "",
  loading,
  updating = false,
}: {
  toolbar?: ReactNode;
  footer?: ReactNode;
  children: ReactNode;
  table?: boolean;
  scrollKey?: string;
  layoutKey?: string;
  className?: string;
  loading?: boolean;
  updating?: boolean;
}) {
  const parentLoading = useContext(ListDataLoadingContext);
  const pending = loading ?? parentLoading;
  const scroller = useRef<HTMLDivElement>(null);
  const statusBar = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const footer = statusBar.current;
    const shell = footer?.closest<HTMLElement>(".app-shell");
    if (!footer || !shell) return;
    const alignSidebar = () => {
      const height = footer.getBoundingClientRect().height;
      // Settings tabs stay mounted; hidden footers must not replace the active height.
      if (height > 0)
        shell.style.setProperty("--admin-footer-height", `${height}px`);
    };
    alignSidebar();
    const observer = new ResizeObserver(alignSidebar);
    observer.observe(footer);
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(() => {
    if (scroller.current) scroller.current.scrollTop = 0;
  }, [scrollKey]);
  const content = (
    <section
      className={`page-panel${table ? " table-panel" : ""} ${className}`}
      aria-busy={pending || updating || undefined}
    >
      {toolbar && <div className="panel-toolbar">{toolbar}</div>}
      {updating && (
        <span role="status" className="sr-only">
          正在更新数据
        </span>
      )}
      <div
        className="panel-scroll"
        ref={scroller}
        tabIndex={0}
        role="region"
        aria-label="页面内容"
      >
        {children}
      </div>
      <footer className="panel-footer" ref={statusBar}>
        {footer ?? <span className="panel-status">数据实时同步</span>}
      </footer>
    </section>
  );
  const panel = table ? (
    <ListLayoutProvider
      storageKey={`${window.location.pathname}:${layoutKey ?? "main"}`}
    >
      {content}
    </ListLayoutProvider>
  ) : (
    content
  );
  return (
    <ListDataLoadingContext.Provider value={pending}>
      {panel}
    </ListDataLoadingContext.Provider>
  );
}

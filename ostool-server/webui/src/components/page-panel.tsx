import { useLayoutEffect, useRef, type ReactNode } from "react";
import { ListLayoutProvider } from "./list-layout";

/** Fixed header/footer with a single independently scrolling content region. */
export function PagePanel({
  toolbar,
  footer,
  children,
  table = false,
  scrollKey,
  layoutKey,
}: {
  toolbar?: ReactNode;
  footer?: ReactNode;
  children: ReactNode;
  table?: boolean;
  scrollKey?: string;
  layoutKey?: string;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (scroller.current) scroller.current.scrollTop = 0;
  }, [scrollKey]);
  const content = (
    <section className={`page-panel${table ? " table-panel" : ""}`}>
      {toolbar && <div className="panel-toolbar">{toolbar}</div>}
      <div
        className="panel-scroll"
        ref={scroller}
        tabIndex={0}
        role="region"
        aria-label="页面内容"
      >
        {children}
      </div>
      <footer className="panel-footer">
        {footer ?? <span className="panel-status">数据实时同步</span>}
      </footer>
    </section>
  );
  return table ? (
    <ListLayoutProvider
      storageKey={`${window.location.pathname}:${layoutKey ?? "main"}`}
    >
      {content}
    </ListLayoutProvider>
  ) : (
    content
  );
}

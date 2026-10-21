import { lazy, StrictMode, Suspense } from "react";
import { createRoot } from "react-dom/client";
import "./style.css";
const App = lazy(() => import("./App"));
const PublicApp = lazy(() => import("./PublicApp"));
createRoot(document.getElementById("app")!).render(
  <StrictMode>
    <Suspense
      fallback={
        <div className="admin-gate" role="status">
          正在加载…
        </div>
      }
    >
      {window.location.pathname.startsWith("/admin") ? <App /> : <PublicApp />}
    </Suspense>
  </StrictMode>,
);

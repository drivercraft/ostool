import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@/styles/global.css";
import "@/apps/web/portal.css";
import PublicApp from "@/apps/web/App";
import { configureAuthSurface } from "@/auth-session";
configureAuthSurface("website");
createRoot(document.getElementById("app")!).render(
  <StrictMode>
    <PublicApp />
  </StrictMode>,
);

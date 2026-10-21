import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "@/apps/admin/App";
import "@/styles/global.css";
import "@/apps/admin/login.css";
import { configureAuthSurface } from "@/auth-session";
configureAuthSurface("admin");

createRoot(document.getElementById("app")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./style.css";
import PublicApp from "./PublicApp";
import { configureAuthSurface } from "./auth-session";
configureAuthSurface("website");
createRoot(document.getElementById("app")!).render(
  <StrictMode>
    <PublicApp />
  </StrictMode>,
);

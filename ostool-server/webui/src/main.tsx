import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import PublicApp from "./PublicApp";
import "./style.css";
createRoot(document.getElementById("app")!).render(
  <StrictMode>
    {window.location.pathname.startsWith("/admin") ? <App /> : <PublicApp />}
  </StrictMode>,
);

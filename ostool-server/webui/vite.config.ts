import { fileURLToPath, URL } from "node:url";

import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

const outDir = process.env.OSTOOL_SERVER_WEB_DIST_DIR ?? "../web/dist";

export default defineConfig({
  base: "/",
  plugins: [
    react(),
    tailwindcss(),
    {
      name: "independent-admin-entry",
      configureServer(server) {
        server.middlewares.use((request, _response, next) => {
          const pathname = request.url?.split("?")[0] ?? "";
          if (
            (pathname === "/admin" || pathname.startsWith("/admin/")) &&
            request.headers.accept?.includes("text/html")
          ) {
            request.url = "/admin/index.html";
          }
          next();
        });
      },
    },
  ],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:4175",
      "/oauth": "http://127.0.0.1:4175",
    },
  },
  build: {
    outDir,
    emptyOutDir: true,
    rollupOptions: {
      input: {
        website: fileURLToPath(new URL("./index.html", import.meta.url)),
        admin: fileURLToPath(new URL("./admin/index.html", import.meta.url)),
      },
    },
  },
  test: {
    environment: "jsdom",
    exclude: ["tests/e2e/**", "**/node_modules/**", "**/dist/**"],
  },
});

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
// @ts-expect-error type error without @types/node package
import { fileURLToPath } from "node:url";

// Separate build of the remote LAN SPA (V1-FR-010), served by the host's HTTP server. It is not part of the Tauri
// bundle and must not import @tauri-apps/* or src/modules/*. Output contract with the Rust server:
//   dist-lan/index.html and every other file under dist-lan/assets/, absolute /assets/... URLs.
// The server sends a strict CSP (script-src 'self'; style-src 'self'), so the HTML has no inline script or style:
// the modulepreload polyfill (an inline helper) is off and no asset is inlined as a data: URL.
export default defineConfig({
  root: fileURLToPath(new URL("./lan-web", import.meta.url)),
  base: "/",
  publicDir: false,
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@dgos/ui": fileURLToPath(new URL("./src/ui", import.meta.url)) },
  },
  build: {
    outDir: fileURLToPath(new URL("./dist-lan", import.meta.url)),
    emptyOutDir: true,
    assetsDir: "assets",
    assetsInlineLimit: 0,
    cssCodeSplit: false,
    modulePreload: { polyfill: false },
    target: "es2020",
  },
});

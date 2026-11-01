import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
// @ts-expect-error type error without @types/node package
import { fileURLToPath } from "node:url";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react(), tailwindcss()],

  // `@dgos/ui` is a path alias to src/ui, not a package or workspace (D066). Keep in sync with
  // tsconfig.json `paths` and vitest.config.ts.
  resolve: {
    alias: { "@dgos/ui": fileURLToPath(new URL("./src/ui", import.meta.url)) },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      // Local herdr worktrees contain several complete copies of this
      // repository. Watching them makes Vite retain a graph for every copy
      // and can grow the dev Node process until it reaches the V8 heap limit.
      ignored: [
        "**/src-tauri/**",
        "**/.herdr-team/**",
        "**/.vitest/**",
        "**/target/**",
        "**/dist/**",
        "**/dist-lan/**",
      ],
    },
  },
}));

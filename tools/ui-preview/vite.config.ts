import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';
import { readFile } from 'node:fs/promises';
const root = fileURLToPath(new URL('./', import.meta.url));
const repo = fileURLToPath(new URL('../../', import.meta.url));
const evidence: Record<string, string> = {
  '/evidence/implementation': `${repo}docs/02-产品与版本/当前版本/V1-实现状态.md`,
  '/evidence/xterm-source': `${repo}tests/fixtures/terminal-accessibility.html`,
};
export default defineConfig({
  root, plugins: [react(), tailwindcss(), {
    name: 'preview-host-boundary',
    configureServer(server) {
      server.middlewares.use((_request, response, next) => {
        response.setHeader('Content-Security-Policy', "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self' ws://127.0.0.1:5197; frame-src 'self'; object-src 'none'; base-uri 'none'");
        const route = (_request.url ?? '').split('?')[0];
        const file = Object.hasOwn(evidence, route) ? evidence[route] : undefined;
        if (file) {
          response.setHeader('Content-Type', 'text/plain; charset=utf-8');
          void readFile(file, 'utf8').then(text => response.end(text), next);
          return;
        }
        next();
      });
    },
  }],
  resolve: { alias: [{ find: /^@tauri-apps\/.*/, replacement: `${root}host.ts` }, { find: '@dgos/ui', replacement: `${repo}src/ui` }] },
  server: { host: '127.0.0.1', port: 5197, strictPort: true, fs: { allow: [repo] }, watch: { ignored: ['**/src-tauri/**', '**/.herdr-team/**', '**/target/**', '**/dist/**'] } },
  // This config is a development server only; product builds keep their own entry/config.
  build: { rollupOptions: { input: `${root}index.html` }, outDir: `${root}.build` },
});

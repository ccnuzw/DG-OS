import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  // Keep in sync with vite.config.ts and tsconfig.json `paths` (D066).
  resolve: {
    alias: { '@dgos/ui': fileURLToPath(new URL('./src/ui', import.meta.url)) },
  },
  test: {
    environment: 'jsdom',
    include: ['tests/**/*.test.tsx'],
    // jsdom gaps that Radix needs; see tests/setup.ts for the reason of each entry.
    setupFiles: ['tests/setup.ts'],
    clearMocks: true,
  },
});

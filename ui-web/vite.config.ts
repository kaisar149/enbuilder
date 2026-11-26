import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

// `npm run dev -w ui-web` serves the UI on :5173 and forwards the WebSocket to a running wb-server (default port 8787).
const server = process.env.WB_SERVER ?? 'http://127.0.0.1:8787';

export default defineConfig({
  plugins: [react()],
  base: './',
  server: { proxy: { '/ws': { target: server, ws: true, changeOrigin: false } } },
  build: { outDir: 'dist', sourcemap: true },
  test: { environment: 'node', testTimeout: 60000, hookTimeout: 60000, include: ['test/**/*.test.{ts,tsx}'] },
});

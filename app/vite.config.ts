import { defineConfig } from 'vite';
import { resolve } from 'node:path';

export default defineConfig({
  base: './',
  // Acceptance fixtures live in app/public for workspace-only checks.  They
  // must never be copied into the production frontend (and then embedded in
  // the Tauri executable) because that would ship test audio and duplicate
  // model resources with every App install.
  publicDir: false,
  build: {
    rollupOptions: {
      input: {
        index: resolve(process.cwd(), 'index.html'),
        headless: resolve(process.cwd(), 'headless.html'),
      },
    },
  },
  worker: {
    format: 'es',
  },
});

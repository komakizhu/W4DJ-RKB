import { defineConfig } from 'vite';
import { resolve } from 'node:path';

export default defineConfig({
  base: './',
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

import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { mockApiPlugin } from './mock/vite-plugin.ts';

const uiSrc = fileURLToPath(new URL('./src', import.meta.url));

// Colour tokens are kept in web/src/tokens.css (moved from the retired ui/ probe, ADR 0036); an alias keeps one import path.
export default defineConfig({
  plugins: [svelte(), mockApiPlugin()],
  resolve: {
    alias: { $tokens: uiSrc },
    // Vitest runs in Node: use the browser entry points of Svelte.
    conditions: process.env.VITEST ? ['browser'] : undefined,
  },
  server: { host: '127.0.0.1', fs: { allow: ['..'] } },
  build: { target: 'es2022', outDir: 'dist', emptyOutDir: true, cssCodeSplit: false },
  test: {
    environment: 'jsdom',
    include: ['tests/**/*.test.ts'],
    setupFiles: ['tests/setup.ts'],
  },
});

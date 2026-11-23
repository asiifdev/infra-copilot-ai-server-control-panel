import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { mockApiPlugin } from './mock/vite-plugin.ts';

const uiSrc = fileURLToPath(new URL('../ui/src', import.meta.url));

// Colour tokens are reused from the prototype (ADR 0029): an alias, never a copy.
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

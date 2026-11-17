import { readFileSync } from 'node:fs';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';

/** The version the packages carry: the workspace version in the root Cargo.toml. */
const APP_VERSION = /^version = "([^"]+)"$/m.exec(
  readFileSync(new URL('../../Cargo.toml', import.meta.url), 'utf8'),
)?.[1];

if (!APP_VERSION) {
  throw new Error('The workspace version is missing from Cargo.toml.');
}

export default defineConfig({
  plugins: [svelte()],
  define: { __APP_VERSION__: JSON.stringify(APP_VERSION) },
  // Tauri expects a fixed dev port and no screen clearing.
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: 'es2022', chunkSizeWarningLimit: 700 },
  resolve: process.env.VITEST ? { conditions: ['browser'] } : undefined,
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
    setupFiles: ['src/test-setup.ts'],
  },
});

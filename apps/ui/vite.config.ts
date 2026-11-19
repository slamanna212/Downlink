import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

// The Tauri app expects the dev server on exactly http://localhost:1420.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: 'localhost' },
  build: { target: 'es2022', sourcemap: false },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    css: false,
    include: ['src/**/*.test.{ts,tsx}'],
  },
});

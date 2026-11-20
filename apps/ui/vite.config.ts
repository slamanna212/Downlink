import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import { visualizer } from 'rollup-plugin-visualizer';

// The Tauri app expects the dev server on exactly http://localhost:1420.
export default defineConfig({
  // `pnpm analyze` writes dist/stats.html (treemap, gzip sizes) for bundle review.
  plugins: [react(), ...(process.env.ANALYZE ? [visualizer({ filename: 'dist/stats.html', gzipSize: true, template: 'treemap' })] : [])],
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

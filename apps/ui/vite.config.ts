import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import { visualizer } from 'rollup-plugin-visualizer';

// The Tauri app expects the dev server on exactly http://localhost:1420.
export default defineConfig({
  // `pnpm analyze` writes dist/stats.html (treemap, gzip sizes) for bundle review.
  plugins: [react(), ...(process.env.ANALYZE ? [visualizer({ filename: 'dist/stats.html', gzipSize: true, template: 'treemap' })] : [])],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: 'localhost' },
  // Pre-bundle every runtime dep at startup. If Vite discovers one mid-session (e.g. behind a lazy
  // route) it re-optimizes with a new hash, and the already-loaded webview ends up with two React
  // copies ("dispatcher is null" on the first hook). Add new bare imports here.
  optimizeDeps: {
    include: [
      '@mantine/core',
      '@mantine/hooks',
      '@mantine/notifications',
      '@mantine/spotlight',
      '@noriginmedia/norigin-spatial-navigation',
      '@tabler/icons-react',
      '@tanstack/react-query',
      '@tanstack/react-virtual',
      '@tauri-apps/api/core',
      '@tauri-apps/api/event',
      '@tauri-apps/api/window',
      'dayjs',
      'dayjs/plugin/timezone',
      'dayjs/plugin/utc',
      'react',
      'react-dom',
      'react-dom/client',
      'react/jsx-runtime',
      'react/jsx-dev-runtime',
      'react-markdown',
      'react-router',
      'remark-gfm',
      'zustand',
      'zustand/react/shallow',
    ],
  },
  build: { target: 'es2022', sourcemap: false },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    css: false,
    include: ['src/**/*.test.{ts,tsx}'],
  },
});

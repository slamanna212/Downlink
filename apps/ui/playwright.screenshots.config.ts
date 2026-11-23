import { defineConfig } from '@playwright/test';

// Marketing screenshots (`pnpm screenshots`): production build + in-browser mock backend, synthetic data only.
export default defineConfig({
  testDir: 'tests/screenshots',
  timeout: 60_000,
  reporter: [['list']],
  use: { baseURL: 'http://localhost:4173', viewport: { width: 1600, height: 1000 }, deviceScaleFactor: 2 },
  webServer: {
    command: 'pnpm build && pnpm exec vite preview --port 4173 --strictPort',
    url: 'http://localhost:4173',
    reuseExistingServer: true,
    timeout: 120_000,
  },
});

import { defineConfig, devices } from '@playwright/test'

/**
 * MessageHub against a real zone through the Vite zone proxy. Start the dev
 * server first:
 *
 *   VITE_CP_USE_MOCK=false VITE_ZONE_PROXY=https://127.0.0.1 \
 *   VITE_ZONE_HOST=sys.test.buckyos.io pnpm run dev --host 127.0.0.1 --port 4175
 *
 * then `MESSAGEHUB_REAL_E2E=1 pnpm exec playwright test --config=playwright.real.config.ts`.
 */
export default defineConfig({
  testDir: './tests/e2e/real',
  outputDir: './test-results-real',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 120_000,
  expect: { timeout: 15_000 },
  use: {
    baseURL: process.env.MESSAGEHUB_REAL_BASE_URL || 'http://127.0.0.1:4175',
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  projects: [
    {
      name: 'chromium-real-zone',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
})

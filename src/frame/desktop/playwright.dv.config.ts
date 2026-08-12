import { defineConfig, devices } from '@playwright/test'

const zoneHost = process.env.BUCKYOS_TEST_ZONE_HOST || 'test.buckyos.io'

export default defineConfig({
  testDir: './tests/dv',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 120_000,
  expect: { timeout: 15_000 },
  use: {
    baseURL: process.env.BUCKYOS_UI_DV_BASE_URL || `https://sys.${zoneHost}`,
    ignoreHTTPSErrors: true,
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

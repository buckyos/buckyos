import { defineConfig, devices } from '@playwright/test'

const PORT = 4020
const BASE_URL = process.env.PLAYWRIGHT_BASE_URL || `http://127.0.0.1:${PORT}`
const webServer = process.env.PLAYWRIGHT_BASE_URL
  ? undefined
  : {
      command: 'pnpm dev:mock',
      cwd: '/home/aa/app/base/buckyos/src/frame/control_panel/web',
      url: BASE_URL,
      reuseExistingServer: true,
      timeout: 60_000,
    }

export default defineConfig({
  testDir: '.',
  testMatch: ['*.spec.ts'],
  fullyParallel: false,
  retries: 0,
  timeout: 45_000,
  expect: {
    timeout: 10_000,
  },
  use: {
    baseURL: BASE_URL,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'off',
  },
  webServer,
  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1440, height: 960 },
      },
    },
  ],
})

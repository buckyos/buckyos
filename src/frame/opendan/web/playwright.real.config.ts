import { defineConfig, devices } from '@playwright/test'

// Smoke against a running opendan that serves the built dist (`--web dist`):
//   OPENDAN_URL=http://127.0.0.1:4060 pnpm exec playwright test -c playwright.real.config.ts
export default defineConfig({
  testDir: './tests/real',
  use: {
    baseURL: process.env.OPENDAN_URL ?? 'http://127.0.0.1:4060',
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
})

import { defineConfig, devices } from '@playwright/test'

export default defineConfig({
  testDir: './tests/e2e',
  fullyParallel: true,
  use: {
    baseURL: 'http://127.0.0.1:4184',
    screenshot: 'only-on-failure',
  },
  webServer: {
    command: 'pnpm run dev --host 127.0.0.1 --port 4184',
    port: 4184,
    reuseExistingServer: true,
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
})

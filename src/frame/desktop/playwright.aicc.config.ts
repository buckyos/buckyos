import { defineConfig } from '@playwright/test'
import base from './playwright.config'

export default defineConfig({
  ...base,
  outputDir: 'test-results-aicc',
  testMatch: ['aicc-models.spec.ts', 'aicc-routing.spec.ts'],
  webServer: {
    command: 'pnpm run dev --host 127.0.0.1 --port 4173',
    env: { VITE_CP_USE_MOCK: 'true' },
    port: 4173,
    reuseExistingServer: !process.env.CI,
  },
})

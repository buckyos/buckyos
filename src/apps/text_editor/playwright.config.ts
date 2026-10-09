import { defineConfig } from '@playwright/test'
export default defineConfig({ testDir: './tests/e2e', fullyParallel: false, workers: 1, use: { baseURL: 'http://127.0.0.1:5178', headless: true, trace: 'retain-on-failure' }, webServer: [
  { command: 'node tests/harness/nfsp.mjs', port: 3260, reuseExistingServer: false },
  { command: 'pnpm exec vite --mode test --host 127.0.0.1', port: 5178, reuseExistingServer: false },
] })

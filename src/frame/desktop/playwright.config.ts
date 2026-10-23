import { defineConfig, devices } from '@playwright/test'

const filesAuto = process.env.FB_NFSP_AUTO === '1'
const port = filesAuto ? 4178 : 4173

export default defineConfig({
  testDir: './tests/e2e',
  fullyParallel: true,
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    screenshot: 'only-on-failure',
    trace: 'on-first-retry',
  },
  webServer: {
    command: `pnpm run dev --host 127.0.0.1 --port ${port} --strictPort`,
    port,
    reuseExistingServer: !filesAuto,
    ...(filesAuto ? { env: { VITE_NFS_PROXY: 'http://127.0.0.1:3262', VITE_CP_USE_MOCK: 'true' } } : {}),
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
})

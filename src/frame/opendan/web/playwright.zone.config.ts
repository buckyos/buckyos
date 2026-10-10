import { defineConfig, devices } from '@playwright/test'

// Against the WebUI served by a zone's gateway (SSO login, session token from the zone):
//   pnpm exec playwright test -c playwright.zone.config.ts
// Env: OPENDAN_ZONE_URL (the agent's app host `https://<agent name>.<zone>`, for the zone
//      owner's agent; default https://jarvis.test.buckyos.io), OPENDAN_ZONE_IP (127.0.0.1),
//      OPENDAN_ZONE_USER (devtest, the agent's owner), OPENDAN_ZONE_PASSWORD (bucky2025).
const url = new URL(process.env.OPENDAN_ZONE_URL ?? 'https://jarvis.test.buckyos.io')
const zone = url.hostname.split('.').slice(1).join('.')
const ip = process.env.OPENDAN_ZONE_IP ?? '127.0.0.1'

export default defineConfig({
  testDir: './tests/zone',
  timeout: 90_000,
  outputDir: 'test-results-zone',
  use: {
    baseURL: url.origin,
    ignoreHTTPSErrors: true,
    screenshot: 'on',
    launchOptions: { args: [`--host-resolver-rules=MAP ${zone} ${ip}, MAP *.${zone} ${ip}`] },
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
})

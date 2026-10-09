import { execFileSync } from 'node:child_process'
import { resolve } from 'node:path'
import { defineConfig, devices } from '@playwright/test'

/**
 * HomeStation end-to-end suite against the REAL homestation service:
 *
 *   (cd ../.. && cargo build -p homestation --example devnet)
 *   pnpm exec playwright test --config=playwright.homestation.config.ts
 *
 * Starts the seeded `devnet` example (me on :4231 with token tok-me; alice, bob, sarah, index on
 * :4232–4235; a fixture RSS site on :4236) with a fresh data folder, and Vite with
 * HS_BACKEND=http://127.0.0.1:4231. The Desktop shell stays on its mock runtime; the HomeStation app
 * talks to the service through the dev override of src/app/homestation/api/transport.ts (?hsDevToken=tok-me).
 *
 * Environment: HS_DEVNET_BIN=<path> selects the devnet binary (default <cargo target>/debug/examples/devnet).
 */

function freePort(): string {
  return execFileSync(process.execPath, ['-e', 'const s=require("net").createServer().listen(0,"127.0.0.1",()=>{console.log(s.address().port);s.close()})'], { encoding: 'utf8' }).trim()
}

process.env.HS_REAL_E2E = '1'
process.env.HS_E2E_WEB_PORT ??= freePort()
const devnet = process.env.HS_DEVNET_BIN ?? resolve(process.env.CARGO_TARGET_DIR ?? resolve(import.meta.dirname, '../../target'), 'debug/examples/devnet')
const dataDir = resolve(import.meta.dirname, 'test-results/homestation-devnet')
const backend = 'http://127.0.0.1:4231'
const web = `http://127.0.0.1:${process.env.HS_E2E_WEB_PORT}`

export default defineConfig({
  testDir: './tests/e2e/pages',
  testMatch: 'homestation.real.spec.ts',
  outputDir: './test-results/homestation-real',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 90_000,
  expect: { timeout: 15_000 },
  use: {
    baseURL: web,
    viewport: { width: 1440, height: 900 },
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  webServer: [
    {
      command: `"${devnet}" --port 4231 --data-dir "${dataDir}" --fresh`,
      url: `${backend}/healthz`,
      wait: { stdout: /"seeded":true/ },
      reuseExistingServer: false,
      timeout: 120_000,
      stdout: 'pipe',
      stderr: 'pipe',
    },
    {
      command: `pnpm exec vite --host 127.0.0.1 --port ${process.env.HS_E2E_WEB_PORT} --strictPort`,
      url: web,
      reuseExistingServer: false,
      timeout: 120_000,
      env: { HS_BACKEND: backend },
    },
  ],
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 } } }],
})

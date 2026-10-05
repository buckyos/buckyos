import { execFileSync } from 'node:child_process'
import { defineConfig, devices } from '@playwright/test'

/**
 * AI Workspace end-to-end suite against the REAL backend (no mock):
 *
 *   pnpm exec playwright test --config=playwright.aiworkspace.config.ts
 *
 * It builds `aiworkspace` (cargo build -p aiworkspace), starts it standalone on a free port with a
 * temporary data folder and two test identities (tok-alice / tok-bob), and starts Vite with
 * AIWS_BACKEND pointing at it. The Desktop shell itself stays on its mock runtime; only the AI
 * Workspace app talks to the backend, through the dev override of src/app/aiworkspace/api/transport.ts.
 * For the offline specs it also makes a production build and serves it with `vite preview` (same
 * backend forward): the service worker exists only there.
 *
 * Environment: AIWS_BIN=<path> skips the cargo build and uses that binary;
 * cargo must be on PATH otherwise (e.g. PATH=/tmp/dev-cache-root/cargo/bin:$PATH).
 */

function freePort(): string {
  return execFileSync(process.execPath, ['-e', 'const s=require("net").createServer().listen(0,"127.0.0.1",()=>{console.log(s.address().port);s.close()})'], { encoding: 'utf8' }).trim()
}

// Chosen once in the runner process; workers inherit the environment.
process.env.AIWS_E2E_BACKEND_PORT ??= freePort()
process.env.AIWS_E2E_WEB_PORT ??= freePort()
process.env.AIWS_E2E_PREVIEW_PORT ??= freePort()
const backend = `http://127.0.0.1:${process.env.AIWS_E2E_BACKEND_PORT}`
const preview = `http://127.0.0.1:${process.env.AIWS_E2E_PREVIEW_PORT}`
const previewDir = 'test-results/aiworkspace-dist'
const web = `http://127.0.0.1:${process.env.AIWS_E2E_WEB_PORT}`
process.env.AIWS_E2E_BACKEND = backend

export default defineConfig({
  testDir: './tests/aiworkspace',
  outputDir: './test-results/aiworkspace',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 90_000,
  expect: { timeout: 15_000 },
  use: {
    baseURL: web,
    viewport: { width: 1600, height: 1000 },
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  webServer: [
    {
      command: 'node tests/aiworkspace/start-backend.mjs',
      url: `${backend}/kapi/aiworkspace/healthz`,
      reuseExistingServer: false,
      timeout: 900_000,
      stdout: 'pipe',
      stderr: 'pipe',
      gracefulShutdown: { signal: 'SIGTERM', timeout: 5000 },
    },
    {
      command: `pnpm exec vite --host 127.0.0.1 --port ${process.env.AIWS_E2E_WEB_PORT} --strictPort`,
      url: web,
      reuseExistingServer: false,
      timeout: 120_000,
      env: { AIWS_BACKEND: backend },
    },
    {
      // The offline specs (offline*.spec.ts) need the service worker, which only a production build
      // has: build once (mock-runtime shell, like the dev server of the other specs) and serve it
      // with `vite preview`. Those specs reach it through a TCP relay they can cut (offline-fixtures.ts).
      command: `pnpm exec vite build --outDir ${previewDir} --emptyOutDir --logLevel warn && pnpm exec vite preview --outDir ${previewDir} --host 127.0.0.1 --port ${process.env.AIWS_E2E_PREVIEW_PORT} --strictPort`,
      url: `${preview}/sw.js`,
      reuseExistingServer: false,
      timeout: 300_000,
      env: { AIWS_BACKEND: backend, VITE_CP_USE_MOCK: 'true' },
    },
  ],
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'], viewport: { width: 1600, height: 1000 } } }],
})

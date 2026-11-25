import { execFileSync } from 'node:child_process'
import { resolve } from 'node:path'
import { defineConfig } from '@playwright/test'

/**
 * The default template of the personal site against fixture data, in a phone viewport. Playwright is the Desktop's
 * (run `pnpm install` in src/frame/desktop once); from the repository root:
 *
 *   pnpm --dir src/frame/desktop exec playwright test -c ../../tools/buckyos-agent/site-template/test/playwright.config.ts
 *
 * or, without pnpm looking at the Desktop's dependencies first:
 *
 *   NODE_PATH=$PWD/src/frame/desktop/node_modules node src/frame/desktop/node_modules/@playwright/test/cli.js test \
 *     -c src/tools/buckyos-agent/site-template/test/playwright.config.ts
 *
 * It starts server.mjs on a free port. Screenshots of every page in both colour schemes land in ./screenshots.
 */

function freePort(): string {
  return execFileSync(process.execPath, ['-e', 'const s=require("net").createServer().listen(0,"127.0.0.1",()=>{console.log(s.address().port);s.close()})'], { encoding: 'utf8' }).trim()
}

process.env.SITE_TEMPLATE_PORT ??= freePort()
const port = process.env.SITE_TEMPLATE_PORT

export default defineConfig({
  testDir: __dirname,
  testMatch: '*.spec.ts',
  outputDir: resolve(__dirname, 'test-results'),
  fullyParallel: true,
  retries: 0,
  reporter: 'list',
  timeout: 30_000,
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    browserName: 'chromium',
    viewport: { width: 375, height: 667 },
    deviceScaleFactor: 2,
    isMobile: true,
    hasTouch: true,
    locale: 'en-US',
    timezoneId: 'Asia/Tokyo',
    screenshot: 'only-on-failure',
  },
  webServer: {
    command: `"${process.execPath}" "${resolve(__dirname, 'server.mjs')}" --port ${port}`,
    url: `http://127.0.0.1:${port}/__test/ping`,
    reuseExistingServer: false,
  },
})

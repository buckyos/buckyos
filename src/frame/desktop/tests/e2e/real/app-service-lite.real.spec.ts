import { expect, test, type Locator, type Page, type TestInfo } from '@playwright/test'
import { writeFile } from 'node:fs/promises'

/**
 * Read-only Desktop Lite DV through a real Gateway.
 * Prerequisites: Text Editor and BuckyOS System Test installed and running, and
 * an existing Agent occupying the Lite slot. No app or Agent is created here.
 *
 * Use the zone's permitted HTTPS origin. For source UI, serve Vite with
 * VITE_CP_USE_MOCK=false and VITE_ZONE_PROXY behind that origin using an isolated
 * TLS fixture; localhost and arbitrary dev ports are rejected by SSO. Supply
 * APP_SERVICE_LITE_REAL_E2E=1, APP_SERVICE_LITE_REAL_BASE_URL, DV_USERNAME and
 * DV_PASSWORD through the environment. Run this file with playwright.real.config.ts
 * and a separate --output directory. Never pass the password on the command line.
 */
const enabled = process.env.APP_SERVICE_LITE_REAL_E2E === '1'
const baseURL = process.env.APP_SERVICE_LITE_REAL_BASE_URL
const username = process.env.DV_USERNAME
const password = process.env.DV_PASSWORD
const resolverRules = process.env.APP_SERVICE_LITE_REAL_RESOLVER_RULES
const certificateSpki = process.env.APP_SERVICE_LITE_REAL_CERTIFICATE_SPKI

test.use({
  baseURL,
  // Login traces can contain credentials and session tokens. Save only the
  // explicit screenshots and sanitized network/console metadata below.
  trace: 'off',
  video: 'off',
  screenshot: 'off',
  actionTimeout: 15_000,
  // Optional isolated fixture routing/trust; never changes host DNS or CA trust.
  launchOptions: { args: [
    ...(resolverRules ? [`--host-resolver-rules=${resolverRules}`] : []),
    ...(certificateSpki ? [`--ignore-certificate-errors-spki-list=${certificateSpki}`] : []),
  ] },
})

function sanitize(value: string): string {
  let safe = password ? value.replaceAll(password, '[REDACTED_PASSWORD]') : value
  safe = safe.replace(/eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g, '[REDACTED_TOKEN]')
  return safe.replace(/https?:\/\/[^\s"')]+/g, text => {
    try { const url = new URL(text); return `${url.origin}${url.pathname}` } catch { return '[URL]' }
  })
}

async function screenshot(page: Page, testInfo: TestInfo, name: string) {
  const path = testInfo.outputPath(`${name}.png`)
  await page.screenshot({ path, fullPage: true, mask: [page.getByLabel('Password', { exact: true })] })
  await testInfo.attach(name, { path, contentType: 'image/png' })
}

async function noHorizontalOverflow(page: Page) {
  const size = await page.evaluate(() => ({
    width: document.documentElement.clientWidth,
    scroll: document.documentElement.scrollWidth,
  }))
  expect(size.scroll).toBeLessThanOrEqual(size.width + 1)
}

async function openDesktopIcon(icon: Locator, touch: boolean) {
  if (touch) await icon.tap()
  else await icon.click()
}

for (const viewport of [
  { name: 'desktop', width: 1440, height: 1000, touch: false },
  { name: 'mobile-375', width: 375, height: 812, touch: true },
]) {
  test.describe(viewport.name, () => {
    test.use({ viewport: { width: viewport.width, height: viewport.height }, hasTouch: viewport.touch })

    test('real Lite app state, installation boundary and occupied Agent slot', async ({ page }, testInfo) => {
      test.skip(!enabled, 'Requires an activated real Lite zone and explicit opt-in.')
      if (!baseURL || !username || !password) throw new Error('Set APP_SERVICE_LITE_REAL_BASE_URL, DV_USERNAME and DV_PASSWORD in the environment.')

      let authenticated = false
      const events: Array<Record<string, unknown>> = []
      const errors: string[] = []
      const rpcMethods = new Set<string>()
      const writes: string[] = []
      page.on('pageerror', error => {
        const message = sanitize(error.message)
        events.push({ type: 'pageerror', authenticated, message })
        errors.push(message)
      })
      page.on('console', message => {
        if (message.type() !== 'error') return
        const text = sanitize(message.text())
        events.push({ type: 'console-error', authenticated, message: text })
        if (authenticated) errors.push(text)
      })
      page.on('requestfailed', request => {
        const message = `${sanitize(request.url())}: ${request.failure()?.errorText}`
        events.push({ type: 'requestfailed', authenticated, message })
        if (authenticated && request.failure()?.errorText !== 'net::ERR_ABORTED') errors.push(message)
      })
      page.on('response', response => {
        const url = sanitize(response.url())
        if (response.status() >= 400) {
          events.push({ type: 'http-error', authenticated, url, status: response.status() })
          if (authenticated) errors.push(`${response.status()} ${url}`)
        }
        if (response.request().method() !== 'POST') return
        try {
          const method = response.request().postDataJSON()?.method
          if (typeof method !== 'string') return
          rpcMethods.add(method)
          events.push({ type: 'rpc', method, status: response.status(), url })
          if (/^(agent\.create|apps\.(install|upgrade|uninstall|start|stop|restart)|app\.install)/.test(method)) writes.push(method)
        } catch { /* Non-JSON request bodies are not recorded. */ }
      })

      try {
        await page.goto('/')
        await page.getByLabel('Username', { exact: true }).fill(username)
        await page.getByLabel('Password', { exact: true }).fill(password)
        await page.getByRole('button', { name: 'Sign In', exact: true }).click()
        const appServiceIcon = page.getByRole('button', { name: 'App Service', exact: true })
        await expect(appServiceIcon).toBeVisible({ timeout: 60_000 })
        authenticated = true
        await openDesktopIcon(appServiceIcon, viewport.touch)
        const app = viewport.touch ? page.getByRole('main').last() : page.getByTestId('window-app-service')
        const backToList = viewport.touch
          ? page.getByRole('button', { name: 'Back', exact: true })
          : app.getByRole('button', { name: 'Back to applications', exact: true })
        await expect(app.getByRole('heading', { name: 'App Service', exact: true })).toBeVisible()
        const systest = app.getByRole('button', { name: /BuckyOS System Test/ })
        const editor = app.getByRole('button', { name: /Text Editor/ })
        await expect(systest).toContainText('Running normally')
        await expect(editor).toContainText('Running normally')
        await expect(app).not.toContainText(/Mock|Prototype|Configuration submitted · deploying/)
        await expect(app.getByRole('heading', { name: 'System Services', exact: true })).toHaveCount(0)
        await expect(app.getByRole('heading', { name: 'Kernel', exact: true })).toHaveCount(0)
        await noHorizontalOverflow(page)
        await screenshot(page, testInfo, '01-app-list')

        await systest.click()
        const runtime = app.getByTestId('app-service-runtime-summary')
        await expect(runtime).toContainText('Script application')
        await expect(runtime.getByTestId('app-service-runtime-status')).toHaveText('Running normally')
        await expect(runtime).not.toContainText(/Docker|Unknown|unavailable/)
        await runtime.scrollIntoViewIfNeeded()
        await screenshot(page, testInfo, '02-script-runtime')

        await backToList.click()
        await editor.click()
        await expect(runtime).toContainText('Static Web application')
        await expect(runtime.getByTestId('app-service-runtime-status')).toHaveText('Running normally')
        await expect(runtime).not.toContainText(/Docker|Unknown|unavailable/)
        await runtime.scrollIntoViewIfNeeded()
        await screenshot(page, testInfo, '03-static-web-runtime')

        await backToList.click()
        await app.getByRole('button', { name: 'Add app', exact: true }).click()
        await expect(app.getByTestId('app-service-lite-notice')).toContainText('BuckyOS Desktop Lite runs no container')
        await expect(app.getByRole('button', { name: 'Choose from Personal Server', exact: true })).toBeDisabled()
        await expect(app).toContainText('This page checks packages; use the BuckyOS CLI to install them.')
        await expect(app.getByRole('button', { name: 'Check source', exact: true })).toHaveCount(0)
        await expect(app.getByTestId('app-service-source-next')).toBeDisabled()
        await expect(app).not.toContainText(/Mock|Prototype/)
        await noHorizontalOverflow(page)
        await screenshot(page, testInfo, '04-real-package-check')
        if (viewport.touch) await page.getByRole('button', { name: 'Minimize', exact: true }).click()
        else await app.getByRole('button', { name: 'Close', exact: true }).click()

        await openDesktopIcon(page.getByRole('button', { name: 'Jarvis', exact: true }), viewport.touch)
        const slot = page.getByTestId('agent-setup-slot-occupied')
        await expect(slot).toContainText('BuckyOS Desktop Lite runs a single Agent')
        const manage = slot.getByRole('button', { name: 'Open Users & Agents', exact: true })
        await expect(manage).toBeVisible()
        const layout = await manage.evaluate(element => {
          const range = document.createRange()
          range.selectNodeContents(element)
          const lines = [...range.getClientRects()].filter(rect => rect.width > 0 && rect.height > 0)
          const box = element.getBoundingClientRect()
          const previous = element.previousElementSibling?.getBoundingClientRect()
          return { lines: new Set(lines.map(rect => Math.round(rect.top))).size, x: box.x, right: box.right, top: box.top, previousBottom: previous?.bottom }
        })
        expect(layout.lines).toBe(1)
        expect(layout.top).toBeGreaterThanOrEqual(layout.previousBottom ?? 0)
        expect(layout.x).toBeGreaterThanOrEqual(0)
        expect(layout.right).toBeLessThanOrEqual(viewport.width)
        await noHorizontalOverflow(page)
        await screenshot(page, testInfo, '05-agent-slot')

        expect([...rpcMethods]).toEqual(expect.arrayContaining(['apps.list', 'apps.status', 'system.product.get']))
        expect(writes).toEqual([])
        expect(errors).toEqual([])
      } catch (error) {
        // Playwright's automatic failure snapshot includes input values.
        await page.getByLabel('Password', { exact: true }).fill('', { timeout: 1_000 }).catch(() => {})
        await screenshot(page, testInfo, 'failure').catch(() => {})
        throw error
      } finally {
        const path = testInfo.outputPath('network-and-console.json')
        await writeFile(path, JSON.stringify({ events, errors, writes, rpcMethods: [...rpcMethods] }, null, 2))
        await testInfo.attach('sanitized-network-and-console', {
          path,
          contentType: 'application/json',
        })
      }
    })
  })
}

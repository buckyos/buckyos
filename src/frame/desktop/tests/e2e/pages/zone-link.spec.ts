import { expect, test, type Page } from '@playwright/test'
import { supportedLocales } from '../../../src/models/ui'
import { seedDesktopFolder } from '../fixtures/desktop'

declare global {
  interface Window {
    __mockZoneLink: { setDown(down: boolean): void }
    zoneLinkFixture: { version: string; error: string | null }
  }
}

const setDown = (page: Page, down: boolean) => page.evaluate((value) => window.__mockZoneLink.setDown(value), down)

async function openTaskbar(page: Page) {
  await page.setViewportSize({ width: 1280, height: 800 })
  await page.goto('/?scenario=normal')
  await page.getByRole('button', { name: 'BuckyOS' }).click()
  const status = page.getByTestId('taskbar-status')
  await expect(status).toContainText(/CPU\s*\d+%/)
  return status
}

test('the offline mark follows the Zone, not the system network', async ({ page, context }, testInfo) => {
  const status = await openTaskbar(page)
  await context.setOffline(true)
  expect(await page.evaluate(() => navigator.onLine)).toBe(false)
  await page.waitForTimeout(6_500)
  await expect(status).toContainText('CPU')

  await setDown(page, true)
  await page.waitForTimeout(2_500)
  await expect(status).toContainText('CPU')
  await setDown(page, false)
  await page.waitForTimeout(5_000)
  await expect(status).toContainText('CPU')

  await setDown(page, true)
  await expect(status).toHaveText('Zoneoffline', { timeout: 10_000 })
  await status.screenshot({ path: testInfo.outputPath('taskbar-unreachable.png') })
  await page.mouse.move(900, 400)
  await page.getByRole('button', { name: 'BuckyOS' }).hover()
  await expect(page.getByTestId('taskbar-profile-card')).toContainText('Zone unreachable')
  await setDown(page, false)
  await expect(status).toContainText('CPU', { timeout: 3_000 })
  await expect(page.getByTestId('taskbar-profile-card')).toContainText('SessionBrowser')
  await expect(page.getByTestId('taskbar-profile-card')).not.toContainText('Relay')
})

test('a phone marks the connection dot', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto('/?scenario=normal')
  const dot = page.getByTestId('status-connection-dot')
  await expect(dot).toHaveAttribute('style', /--cp-success/)
  await setDown(page, true)
  await expect(dot).toHaveAttribute('style', /--cp-danger/, { timeout: 10_000 })
  await setDown(page, false)
  await expect(dot).toHaveAttribute('style', /--cp-success/, { timeout: 3_000 })
})

for (const locale of supportedLocales) {
  test(`the taskbar's two lines fit in ${locale}`, async ({ page }) => {
    await page.addInitScript((value) => localStorage.setItem('buckyos.prototype.locale.v1', value), locale)
    await page.setViewportSize({ width: 1280, height: 800 })
    await page.goto('/?scenario=normal')
    await page.getByRole('button', { name: 'BuckyOS' }).click()
    await setDown(page, true)
    const status = page.getByTestId('taskbar-status')
    await expect(status).toContainText('Zone', { timeout: 10_000 })
    const cut = await status.locator('.truncate').evaluateAll((labels) => labels.filter((label) => label.scrollWidth > label.clientWidth).map((label) => label.textContent))
    expect(cut).toEqual([])
  })
}

async function openSystemInfo(page: Page) {
  await seedDesktopFolder(page, {}, { 'System Info.widget': { kind: 'file', size: 24, mtime: 1, text: JSON.stringify({ widget: 'system-info' }) } })
  await page.addInitScript(() => { window.zoneLinkFixture = { version: '0.7.2', error: null } })
  await page.route(/\/src\/api\/settings\.ts$/, (route) => route.fulfill({
    contentType: 'text/javascript',
    body: `export * from '/src/api/settings.ts?zone-link-original';
      export async function fetchBuckyOSInfo() {
        const { version, error } = window.zoneLinkFixture;
        return error ? {data: null, error} : {data: { version, build_version: version, release_channel: 'beta', target: 'test' }, error: null};
      }`,
  }))
  await page.clock.install()
  await page.setViewportSize({ width: 1280, height: 860 })
  await page.goto('/?scenario=normal')
  const fields = page.getByTestId('system-info-fields')
  await expect(fields).toContainText('0.7.2')
  return fields
}

test('polling goes on while the system reports no network', async ({ page, context }) => {
  const fields = await openSystemInfo(page)
  await context.setOffline(true)
  await page.evaluate(() => { window.zoneLinkFixture.version = '0.7.3' })
  await page.clock.runFor(61_000)
  await expect(fields).toContainText('0.7.3')
})

test('what failed is read again once the Zone answers, without an online event', async ({ page }) => {
  const fields = await openSystemInfo(page)
  await setDown(page, true)
  await page.evaluate(() => { window.zoneLinkFixture.error = 'down' })
  await page.clock.runFor(61_000)
  await expect(fields).not.toContainText('0.7.2')
  await page.evaluate(() => { window.zoneLinkFixture = { version: '0.7.4', error: null } })
  await page.clock.runFor(5_000)
  await expect(fields).not.toContainText('0.7.4')
  await setDown(page, false)
  await page.clock.runFor(3_000)
  await expect(fields).toContainText('0.7.4')
})

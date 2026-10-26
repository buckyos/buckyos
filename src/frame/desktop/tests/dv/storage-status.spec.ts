import { expect, test, type Response } from '@playwright/test'
import type { ZoneDevicesResponse } from '../../src/api/settings'
import type { SystemMetrics } from '../../src/api/system_metrics'
import { formatBytes } from '../../src/app/settings/components/shared/format'
import { openFromAllApps } from '../e2e/fixtures/desktop'

const username = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const password = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || ''
const gib = 1024 ** 3

function isRpcResponse(response: Response, method: string): boolean {
  const request = response.request()
  if (!response.url().includes('/kapi/control-panel') || request.method() !== 'POST') return false
  const body = request.postDataJSON() as { method?: string; params?: { lite?: boolean } }
  return body.method === method && (method !== 'system.metrics' || body.params?.lite === true)
}

test.describe('Storage status on the DV zone', () => {
  test.skip(!password, 'Set BUCKYOS_TEST_ADMIN_PASSWORD for the zone administrator.')
  test.use({ viewport: { width: 1440, height: 1000 } })

  test('taskbar and Settings report the same local storage capacity', async ({ page }, testInfo) => {
    const pageErrors: string[] = []
    page.on('pageerror', error => pageErrors.push(error.message))

    await page.goto('/')
    await page.getByLabel('Username').fill(username)
    await page.getByLabel('Password').fill(password)
    await page.getByRole('button', { name: 'Sign In', exact: true }).click()
    await expect(page.getByTestId('desktop-taskbar-avatar')).toBeVisible({ timeout: 30_000 })

    const devicesResponse = page.waitForResponse(response => isRpcResponse(response, 'zone.devices.list'))
    await openFromAllApps(page, 'settings')
    const devicesHttp = await devicesResponse
    expect(devicesHttp.ok()).toBe(true)
    const devicesBody = await devicesHttp.json() as { result: ZoneDevicesResponse; error?: unknown }
    expect(devicesBody.error).toBeFalsy()
    const localDevices = devicesBody.result.devices.filter(device => device.is_local)
    expect(localDevices).toHaveLength(1)
    const localDevice = localDevices[0]
    expect(localDevice.online).toBe(true)
    const totalBytes = Number(localDevice.storage?.total_bytes)
    expect(totalBytes).toBeGreaterThan(0)

    const settings = page.getByTestId('window-settings')
    await settings.getByRole('button', { name: 'Cluster Manager', exact: true }).click()
    await settings.getByTestId(`settings-device-${localDevice.name}`).getByRole('button').first().click()
    const detail = settings.getByTestId(`settings-device-detail-${localDevice.name}`)
    const settingsStorage = detail.getByText('Storage', { exact: true }).locator('..')
    await expect(settingsStorage).toContainText(`/ ${formatBytes(totalBytes)} (`)

    const metricsResponse = page.waitForResponse(response => isRpcResponse(response, 'system.metrics'))
    if (await page.getByTestId('desktop-taskbar').getAttribute('aria-hidden') === 'true') {
      await page.getByTestId('desktop-taskbar-avatar').click()
    }
    await page.getByTestId('taskbar-status').click()
    const status = page.getByTestId('taskbar-status-card')
    await expect(status).toBeVisible()
    const metricsHttp = await metricsResponse
    expect(metricsHttp.ok()).toBe(true)
    const metricsBody = await metricsHttp.json() as { result: SystemMetrics; error?: unknown }
    expect(metricsBody.error).toBeFalsy()
    const totalGb = metricsBody.result.disk.totalGb
    expect(totalGb).toBeGreaterThan(0)

    const locale = await page.locator('html').getAttribute('lang') || 'en'
    const taskbarTotal = new Intl.NumberFormat(locale, { maximumFractionDigits: totalGb < 100 ? 1 : 0 }).format(totalGb)
    const taskbarStorage = status.getByText('Storage', { exact: true }).locator('../..')
    await expect(taskbarStorage).toContainText(`/ ${taskbarTotal} GB`)
    expect(pageErrors).toEqual([])
    await testInfo.attach('storage-status', { body: await page.screenshot({ animations: 'disabled' }), contentType: 'image/png' })
    expect(Math.abs(totalGb * gib - totalBytes), 'Taskbar and local Settings capacity must agree within 1 MiB').toBeLessThanOrEqual(1024 ** 2)
  })
})

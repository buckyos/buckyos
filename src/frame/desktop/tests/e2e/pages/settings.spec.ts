import { expect, test, type Page } from '@playwright/test'

async function openSettings(page: Page, item?: string) {
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-settings').click()
  const win = page.getByTestId('window-settings')
  await expect(win.getByTestId('settings-general-page')).toBeVisible()
  if (item) await win.getByRole('button', { name: item, exact: true }).click()
  return win
}

test.describe('settings', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 860 })
  })

  test('general: software and snapshot from the Zone, no intro block, no device info', async ({ page }) => {
    const win = await openSettings(page)
    await expect(win.getByTestId('settings-software-info')).toContainText('0.7.0')
    await expect(win.getByTestId('settings-system-snapshot')).toContainText('2 of 3 online')
    await expect(win.getByText('System information, device details, and support tools.')).toHaveCount(0)
    await expect(win.getByText('Device Info', { exact: true })).toHaveCount(0)

    const download = page.waitForEvent('download')
    await win.getByTestId('settings-export-system-info').click()
    expect((await download).suggestedFilename()).toMatch(/^buckyos-system-info-.*\.json$/)
  })

  test('navigation names the Zone page "Zone Config"', async ({ page }) => {
    const win = await openSettings(page)
    await expect(win.getByRole('button', { name: 'Zone Config', exact: true })).toBeVisible()
    await expect(win.getByRole('button', { name: 'Zone Name & Icon' })).toHaveCount(0)
    await win.getByRole('button', { name: 'Zone Config', exact: true }).click()
    await expect(win.getByTestId('settings-zone-page')).toBeVisible()
  })

  test('appearance: no session management or wallpaper; the icon size persists', async ({ page }) => {
    const win = await openSettings(page, 'Appearance')
    await expect(win.getByTestId('settings-appearance-page')).toBeVisible()
    await expect(win.getByText('Session Management')).toHaveCount(0)
    await expect(win.getByText('Current Session')).toHaveCount(0)
    await expect(win.getByText('Wallpaper')).toHaveCount(0)

    await win.getByTestId('settings-icon-size').selectOption('large')
    await page.reload()
    await page.getByTestId('desktop-app-settings').click()
    await page.getByTestId('window-settings').getByRole('button', { name: 'Appearance', exact: true }).click()
    await expect(page.getByTestId('settings-icon-size')).toHaveValue('large')
  })

  test('cluster: devices with their hardware, add-device notice, certificates', async ({ page }) => {
    const win = await openSettings(page, 'Cluster Manager')
    const devices = win.getByTestId('settings-cluster-devices')
    await expect(devices.getByTestId('settings-device-ood1')).toContainText('This node')
    await expect(devices.getByTestId('settings-device-ood2')).toContainText('Online')
    await expect(devices.getByTestId('settings-device-laptop')).toContainText('Offline')

    await devices.getByTestId('settings-device-ood1').getByRole('button').first().click()
    const detail = devices.getByTestId('settings-device-detail-ood1')
    await expect(detail).toContainText('AMD Ryzen 7 5825U')
    await expect(detail).toContainText('Ubuntu 24.04.1 LTS')
    await expect(detail).toContainText('32 GB')

    await devices.getByTestId('settings-add-device').click()
    await expect(page.getByTestId('settings-add-device-notice')).toContainText('not supported in BuckyOS Desktop')

    await expect(win.getByTestId('settings-cluster-certificates')).toContainText('*.alice.web3.buckyos.ai')
    await expect(win.getByTestId('settings-cluster-certificates')).toContainText('ACME')
    await expect(win.getByTestId('settings-cluster-connectivity')).toContainText('sn.buckyos.ai')
  })

  test('privacy: public access, messaging, data and app access; no device permissions', async ({ page }) => {
    const win = await openSettings(page, 'Privacy')
    await expect(win.getByTestId('settings-privacy-public')).toContainText('Photo Gallery')
    await expect(win.getByTestId('settings-privacy-public')).toContainText('2026 家庭旅行计划')
    await expect(win.getByTestId('settings-privacy-messaging')).toContainText('2 blocked')
    await expect(win.getByTestId('settings-privacy-data')).toContainText('/srv/library/')
    await expect(win.getByTestId('settings-privacy-app-code-assistant@alice')).toContainText('/home/alice/projects')
    await expect(win.getByTestId('settings-privacy-agent-jarvis')).toContainText('telegram')
    await expect(win.getByText('Device & Capability Permissions')).toHaveCount(0)
  })

  test('developer: diagnostics, gateway config files and log download', async ({ page }) => {
    const win = await openSettings(page, 'Developer Mode')
    await expect(win.getByTestId('settings-dev-diagnostics')).toContainText('OOD nodes are reporting')
    await expect(win.getByTestId('settings-dev-diagnostics')).toContainText('TLS Certificate')

    const config = win.getByTestId('settings-dev-config')
    await config.getByRole('button', { name: 'node_gateway.json' }).click()
    await expect(config).toContainText('zone_tls')

    const download = page.waitForEvent('download')
    await win.getByTestId('settings-download-logs').click()
    expect((await download).suggestedFilename()).toBe('buckyos-logs-mock.txt')
    await expect(win.getByText('Coming soon')).toHaveCount(0)
  })
})

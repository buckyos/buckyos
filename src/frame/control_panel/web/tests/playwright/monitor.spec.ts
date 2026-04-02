import { expect, test } from '@playwright/test'

test('desktop monitor smoke works in mock mode', async ({ page }) => {
  await page.goto('/')

  await expect(page.getByTestId('desktop-shortcut-monitor')).toBeVisible()
  await page.getByTestId('desktop-shortcut-monitor').click()

  const monitorWindow = page.getByTestId('desktop-window-monitor')
  await expect(monitorWindow).toBeVisible()
  await expect(page.getByTestId('desktop-window-title-monitor')).toContainText('System Monitor')
  await expect(monitorWindow.getByText('CPU', { exact: true })).toBeVisible()
  await expect(monitorWindow.getByText('Memory', { exact: true })).toBeVisible()
  await expect(monitorWindow.getByText('Storage', { exact: true })).toBeVisible()
  await expect(monitorWindow.getByText('Network', { exact: true })).toBeVisible()
  await expect(monitorWindow.getByText('CPU / Memory trend')).toBeVisible()
  await expect(monitorWindow.getByText('Network throughput trend')).toBeVisible()
  await expect(monitorWindow.getByText('System status')).toBeVisible()
})

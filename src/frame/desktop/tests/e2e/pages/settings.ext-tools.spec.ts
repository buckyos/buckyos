import { expect, test, type Page } from '@playwright/test'

async function openPage(page: Page) {
  await page.setViewportSize({ width: 1280, height: 860 })
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-settings').click()
  const win = page.getByTestId('window-settings')
  await expect(win.getByTestId('settings-general-page')).toBeVisible()
  await win.getByRole('button', { name: 'Extension Tools', exact: true }).click()
  const section = win.getByTestId('ext-tools-page')
  await expect(section).toBeVisible()
  return section
}

test.describe('settings: extension tools', () => {
  test('each package shows what it provides and where each node got to', async ({ page }) => {
    const section = await openPage(page)
    const seeded = section.getByTestId('ext-tools-package-ext-core')
    await expect(seeded).toContainText('Came with BuckyOS')
    await expect(seeded).toContainText('aigc_video')
    await expect(seeded.getByTestId('ext-tools-node-ood1')).toContainText('Active')
    const media = section.getByTestId('ext-tools-package-ext-media')
    await expect(media).toContainText('ffmpeg, ffprobe')
    await expect(media).toContainText('ext-core')
  })

  test('enabling asks first, lists the plan, and the package then shows on the node', async ({ page }) => {
    const section = await openPage(page)
    const media = section.getByTestId('ext-tools-package-ext-media')
    await media.getByRole('button', { name: 'Enable', exact: true }).click()
    const dialog = page.getByTestId('ext-tools-confirm')
    await expect(dialog).toContainText('ext-media 1.2.0')
    await dialog.getByTestId('ext-tools-confirm-ok').click()
    await expect(media.getByTestId('ext-tools-node-ood1')).toContainText('Active')
    await expect(media.getByRole('button', { name: 'Disable', exact: true })).toBeVisible()
  })
})

import { expect, test, type Locator, type Page } from '@playwright/test'

/** The mock control panel's online items (api/settings_mock.ts): ext-core's `chromium` on ood1, ready to begin with. */
interface ExtToolsMock {
  setOnline(node: string, item: string, state: 'missing' | 'failed' | 'ready', error?: string): void
  failNextDownload(item: string, error: string): void
}
const extToolsMock = (page: Page) => ({
  setOnline: (state: 'missing' | 'failed' | 'ready') =>
    page.evaluate((value) => (window as unknown as { __extToolsMock: ExtToolsMock }).__extToolsMock.setOnline('ood1', 'chromium', value), state),
  failNext: (error: string) =>
    page.evaluate((value) => (window as unknown as { __extToolsMock: ExtToolsMock }).__extToolsMock.failNextDownload('chromium', value), error),
})

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

test.describe('settings: extension tools, online items', () => {
  async function chromium(page: Page): Promise<{ section: Locator; core: Locator; row: Locator }> {
    const section = await openPage(page)
    const core = section.getByTestId('ext-tools-package-ext-core')
    const row = core.getByTestId('ext-tools-online-chromium-ood1')
    await expect(row).toHaveAttribute('data-state', 'ready')
    return { section, core, row }
  }

  const refresh = (section: Locator) => section.getByRole('button', { name: 'Refresh' }).click()

  test('an online item shows what it is, where each node got to, and downloads to ready', async ({ page }) => {
    const { section, core, row } = await chromium(page)
    const item = core.getByTestId('ext-tools-online-item-chromium')
    await expect(item).toContainText('159.0.8154.6')
    await expect(item).toContainText('199 MB')
    await expect(item).toContainText('Chrome for Testing: the browser of the Agent Browser')
    await item.getByTestId('ext-tools-online-license').locator('summary').click()
    await expect(item.getByTestId('ext-tools-online-license')).toContainText('BuckyOS does not distribute it')
    await expect(row).toContainText('Ready')
    await expect(core.getByTestId('ext-tools-waiting-chromium')).toHaveCount(0)

    await extToolsMock(page).setOnline('missing')
    await refresh(section)
    await expect(row).toHaveAttribute('data-state', 'missing')
    await expect(row).toContainText('Not downloaded')
    await expect(core.getByTestId('ext-tools-waiting-chromium')).toHaveText(' (waiting for the browser to download)')
    await expect(core.getByTestId('ext-tools-waiting-page-render')).toBeVisible()
    await expect(core.getByTestId('ext-tools-waiting-agent-browser')).toHaveCount(0)

    await row.getByRole('button', { name: 'Download', exact: true }).click()
    await expect(row).toHaveAttribute('data-state', 'requested')
    await expect(row).toContainText('Asked the node')
    await expect(row.getByTestId('ext-tools-online-download')).toHaveCount(0)
    await expect(row).toHaveAttribute('data-state', 'downloading')
    await expect(row).toContainText(/Downloading \d+%/)
    await expect(row.getByRole('progressbar')).toBeVisible()
    await expect(row).toContainText(/MB of 199 MB/)
    await expect(row).toHaveAttribute('data-state', 'ready', { timeout: 15_000 })
    await expect(core.getByTestId('ext-tools-waiting-chromium')).toHaveCount(0)
  })

  test('a failed download says why, and is retried', async ({ page }) => {
    const { section, row } = await chromium(page)
    await extToolsMock(page).setOnline('failed')
    await refresh(section)
    await expect(row).toHaveAttribute('data-state', 'failed')
    await expect(row).toContainText('Download failed')
    await expect(row.getByTestId('ext-tools-online-error')).toHaveText('connection to storage.googleapis.com timed out')

    await extToolsMock(page).failNext('sha256 of the archive does not match')
    await row.getByRole('button', { name: 'Retry', exact: true }).click()
    await expect(row.getByTestId('ext-tools-online-error')).toHaveText('sha256 of the archive does not match', { timeout: 15_000 })
    await expect(row).toHaveAttribute('data-state', 'failed')

    await row.getByRole('button', { name: 'Retry', exact: true }).click()
    await expect(row).toHaveAttribute('data-state', 'ready', { timeout: 15_000 })
    await expect(row.getByTestId('ext-tools-online-error')).toHaveCount(0)
  })

  test('downloading a ready item again says it replaces it, and asks first', async ({ page }) => {
    const { row } = await chromium(page)
    await expect(row).toContainText('replaces the copy as a whole')
    await row.getByRole('button', { name: 'Download again', exact: true }).click()
    const dialog = page.getByTestId('ext-tools-online-again')
    await expect(dialog).toContainText('Download chromium again on ood1?')
    await expect(dialog).toContainText('The copy in use keeps working until then')
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    await expect(dialog).toHaveCount(0)
    await page.waitForTimeout(1200)
    await expect(row).toHaveAttribute('data-state', 'ready')

    await extToolsMock(page).failNext('the download stalled for 60 seconds')
    await row.getByRole('button', { name: 'Download again', exact: true }).click()
    await page.getByTestId('ext-tools-online-again-ok').click()
    await expect(row).toHaveAttribute('data-state', 'requested')
    // Fetching it again failed: the copy in use stays, and says so.
    await expect(row.getByTestId('ext-tools-online-error')).toHaveText('Downloading it again failed: the download stalled for 60 seconds. The copy in use is kept.', { timeout: 15_000 })
    await expect(row).toHaveAttribute('data-state', 'ready')
  })
})

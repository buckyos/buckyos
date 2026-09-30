import { expect, test, type Page } from '@playwright/test'

/**
 * MessageHub × Preview integration (PRD §11.5, §14.2) on the mock runtime.
 *
 * The seeded Alice conversation carries a GIF, a PNG, a WebM screen recording,
 * a QuickTime clip the Runtime cannot play and a PDF (served by the mock
 * `ObjectAccess`), plus a trusted external image.
 */

const ALICE = 'did:buckyos:person:alice'

async function openAlice(page: Page) {
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({
    contentType: 'image/svg+xml',
    body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"><rect width="800" height="600" fill="lightblue"/></svg>',
  }))
  await page.goto(`/messagehub?entityId=${encodeURIComponent(ALICE)}`)
  const history = page.getByTestId('conversation-history')
  await expect(history.getByTestId('attachment-image').filter({ has: page.getByRole('img', { name: 'orbit-loader.gif' }) })).toBeVisible({ timeout: 15_000 })
  return history
}

test('stream shows GIFs animated, videos as Preview thumbnails and files as cards', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  const history = await openAlice(page)

  const gif = history.getByTestId('attachment-image').filter({ has: page.getByRole('img', { name: 'orbit-loader.gif' }) })
  await expect(gif).toHaveAttribute('data-gif', 'animated')
  await expect.poll(() => gif.locator('img').evaluate(img => (img as HTMLImageElement).naturalWidth)).toBe(160)
  await expect(history.getByRole('img', { name: 'harbor-sunset.png', exact: true })).toBeVisible()

  // WebM: no Pipeline plan in the mock catalog → Runtime frame capture.
  const clip = history.getByTestId('attachment-video').filter({ has: page.getByRole('img', { name: 'screen-recording.webm' }) })
  await expect(clip).toHaveAttribute('data-thumbnail', 'ready', { timeout: 20_000 })
  await expect(clip).toHaveAttribute('data-thumbnail-via', 'runtime-capture')
  // QuickTime: the Runtime cannot decode it → Pipeline with purpose `thumbnail`.
  const mov = history.getByTestId('attachment-video').filter({ has: page.getByRole('img', { name: 'interview-cut.mov' }) })
  await expect(mov).toHaveAttribute('data-thumbnail', 'ready', { timeout: 20_000 })
  await expect(mov).toHaveAttribute('data-thumbnail-via', 'pipeline')

  await expect(history.getByTestId('attachment-file')).toContainText('design-brief.pdf')
  await page.screenshot({ path: 'test-results/messagehub-media-stream.png' })
  expect(errors).toEqual([])
})

test('clicking media opens the in-window Preview with the conversation media as its session', async ({ page }) => {
  const history = await openAlice(page)
  await history.getByTestId('attachment-image').filter({ has: page.getByRole('img', { name: 'orbit-loader.gif' }) }).click()

  const viewer = page.getByTestId('messagehub-media-viewer')
  await expect(viewer).toBeVisible()
  const preview = viewer.getByTestId('messagehub-media-preview')
  await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 15_000 })
  await expect(preview).toHaveAttribute('data-renderer', 'image')
  // External image + GIF + PNG + WebM + MOV; the PDF is not part of the media set.
  await expect(preview).toHaveAttribute('data-item-count', '5')
  await expect(preview).toHaveAttribute('data-item-index', '1')
  await page.screenshot({ path: 'test-results/messagehub-media-viewer-gif.png' })

  await page.keyboard.press('PageDown')
  await expect(preview).toHaveAttribute('data-item-index', '2')
  await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 15_000 })
  await page.keyboard.press('PageDown')
  await expect(preview).toHaveAttribute('data-item-index', '3')
  await expect(preview).toHaveAttribute('data-renderer', 'video', { timeout: 15_000 })
  await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 15_000 })

  await page.keyboard.press('Escape')
  await expect(viewer).toHaveCount(0)

  // A file card opens on its own (no media browsing set).
  await history.getByTestId('attachment-file').click()
  await expect(preview).toHaveAttribute('data-item-count', '1')
  await expect(preview).toHaveAttribute('data-renderer', 'pdf', { timeout: 15_000 })
  // The backdrop closes the pop-up too.
  await viewer.click({ position: { x: 10, y: 200 } })
  await expect(viewer).toHaveCount(0)
})

test('a video the Runtime cannot play is converted by the Pipeline inside the pop-up', async ({ page }) => {
  const history = await openAlice(page)
  const mov = history.getByTestId('attachment-video').filter({ has: page.getByRole('img', { name: 'interview-cut.mov' }) })
  await mov.click()
  const preview = page.getByTestId('messagehub-media-preview')
  await expect(preview).toHaveAttribute('data-renderer', 'video', { timeout: 20_000 })
  await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 20_000 })
})

test('media settings: GIF autoplay off shows the first frame; the window target needs the desktop', async ({ page }) => {
  const history = await openAlice(page)
  await page.getByRole('button', { name: 'Media settings', exact: true }).first().click()
  const dialog = page.getByTestId('messagehub-media-settings')
  await expect(dialog.getByRole('radio', { name: 'Preview window' })).toBeDisabled()
  await dialog.getByRole('switch', { name: 'Autoplay GIFs' }).click()
  await dialog.getByRole('button', { name: 'Close', exact: true }).last().click()
  await expect(dialog).toHaveCount(0)
  const gif = history.getByTestId('attachment-image').filter({ has: page.getByRole('img', { name: 'orbit-loader.gif' }) })
  await expect(gif).toHaveAttribute('data-gif', 'still')
  await expect(gif.getByTestId('attachment-gif-still')).toBeVisible()
  await page.reload()
  await expect(page.getByTestId('attachment-gif-still')).toBeVisible({ timeout: 15_000 })
})

test('desktop: the Preview window preference hands attachments to the Preview App', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.addInitScript(() => window.localStorage.setItem('buckyos.messagehub.media.v1', JSON.stringify({ previewTarget: 'window', autoplayGif: true })))
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="80" height="60"/>' }))
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-messagehub').click()
  const hub = page.getByTestId('window-messagehub')
  await hub.getByText('Alice Chen', { exact: true }).first().click()
  const image = hub.getByTestId('attachment-image').filter({ has: page.getByRole('img', { name: 'harbor-sunset.png' }) })
  await expect(image).toBeVisible({ timeout: 15_000 })
  await image.click()
  await expect(hub.getByTestId('messagehub-media-viewer')).toHaveCount(0)
  const previewWindow = page.getByTestId('window-preview')
  await expect(previewWindow).toBeVisible()
  const preview = previewWindow.getByTestId('content-preview')
  await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 15_000 })
  await expect(preview).toHaveAttribute('data-renderer', 'image')
  await expect(preview).toHaveAttribute('data-item-count', '5')
  await expect(preview).toHaveAttribute('data-item-index', '2')
  await page.screenshot({ path: 'test-results/messagehub-media-preview-window.png' })
})

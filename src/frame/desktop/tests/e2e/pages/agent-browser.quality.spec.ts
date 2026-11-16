import { expect, test, type Page } from '@playwright/test'
import { openFromAllApps } from '../fixtures/desktop'

/** How clear the screen's frames are (Agent Browser Protocol §3.2 `pixel_ratio` and `quality`), on a phone of 2.6
 * device pixels a CSS pixel, on the mock Agent Service: the frames follow the screen's density and the connection
 * (`__agentBrowserMock.network` slows the frames down), unless the user fixed a choice in the menu. */

test.use({
  viewport: { width: 412, height: 780 },
  deviceScaleFactor: 2.6,
  hasTouch: true,
  isMobile: true,
})

const AGENT = 'codeassistant'

interface Message { type: string; viewport?: { width: number; height: number; mobile: boolean; pixel_ratio?: number }; pixel_ratio?: number; jpeg_quality?: number; max_fps?: number }
interface MockControl { log: Array<{ message: Message }>; network: { bytesPerSecond: number | null } }
type MockWindow = { __agentBrowserMock: MockControl }

const received = (page: Page, type: string) =>
  page.evaluate((t) => (window as unknown as MockWindow).__agentBrowserMock.log.filter((entry) => entry.message.type === t).map((entry) => entry.message), type)
const slowDown = (page: Page, bytesPerSecond: number | null) =>
  page.evaluate((rate) => { (window as unknown as MockWindow).__agentBrowserMock.network.bytesPerSecond = rate }, bytesPerSecond)

/** `again`: the App remembers the Agent from before and shows its browser at once. */
async function openBrowser(page: Page, again = false) {
  await page.goto('/?scenario=normal')
  await expect(page.getByTestId('mobile-taskbar')).toBeAttached()
  await openFromAllApps(page, 'agent-browser')
  if (!again) await page.getByTestId(`ab-agent-${AGENT}`).tap()
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
  const address = page.getByTestId('ab-address-input')
  await address.tap()
  await address.fill('example.com')
  await address.press('Enter')
  await expect(page.getByTestId('ab-screen-canvas')).toHaveAttribute('data-seq', /\d+/)
}

/** The pixels of the JPEG on the screen's canvas, and the CSS pixels of the frame it shows. */
async function shown(page: Page) {
  const viewport = await page.getByTestId('ab-screen-surface').getAttribute('data-viewport')
  const [width, height] = (viewport ?? '0x0').split('x').map(Number)
  const pixels = await page.getByTestId('ab-screen-canvas').evaluate((canvas: HTMLCanvasElement) => ({ width: canvas.width, height: canvas.height }))
  return { width, height, pixels }
}

test('the frames are as dense as the screen, and follow the connection down and up again', async ({ page }) => {
  test.setTimeout(120_000)
  await openBrowser(page)
  const screen = page.getByTestId('ab-screen')

  // A screen of 2.6: frames of twice the CSS pixels (the most the service paints), drawn at the CSS size.
  await expect.poll(async () => (await received(page, 'hello'))[0]?.viewport).toMatchObject({ width: 412, mobile: true, pixel_ratio: 2 })
  await expect.poll(async () => (await received(page, 'quality'))[0]).toEqual({ type: 'quality', max_fps: 15, jpeg_quality: 70, pixel_ratio: 2 })
  await expect(screen).toHaveAttribute('data-tier', 'high')
  await expect.poll(async () => { const { width, pixels } = await shown(page); return pixels.width === width * 2 }).toBe(true)
  const { width, height, pixels } = await shown(page)
  expect(pixels.height).toBe(height * 2)
  const box = await page.getByTestId('ab-screen-canvas').boundingBox()
  expect(Math.round(box?.width ?? 0)).toBe(412)
  expect(width).toBe(412)

  // The connection slows down to 15 KB/s: the tier goes down at once, to plain pixels and then fewer bytes and frames.
  await slowDown(page, 15_000)
  await expect(screen).toHaveAttribute('data-tier', 'standard', { timeout: 10_000 })
  await expect.poll(async () => (await shown(page)).pixels.width, { timeout: 10_000 }).toBe(412)
  await expect(screen).toHaveAttribute('data-tier', 'minimal', { timeout: 20_000 })
  await expect.poll(async () => (await received(page, 'quality')).at(-1)).toEqual({ type: 'quality', max_fps: 3, jpeg_quality: 30, pixel_ratio: 1 })

  // It is fast again: after a while the tier goes back up, one step at a time, to the screen's density.
  await slowDown(page, null)
  await expect(screen).toHaveAttribute('data-tier', 'saver', { timeout: 20_000 })
  await expect(screen).toHaveAttribute('data-tier', 'high', { timeout: 30_000 })
  await expect.poll(async () => (await received(page, 'quality')).at(-1)).toEqual({ type: 'quality', max_fps: 15, jpeg_quality: 70, pixel_ratio: 2 })
  await expect.poll(async () => (await shown(page)).pixels.width, { timeout: 10_000 }).toBe(824)
})

test('a choice in the menu holds whatever the connection, and is kept on this device', async ({ page }) => {
  test.setTimeout(90_000)
  await openBrowser(page)
  const screen = page.getByTestId('ab-screen')
  await expect(screen).toHaveAttribute('data-tier', 'high')

  await page.getByTestId('ab-menu-button').tap()
  await expect(page.getByTestId('ab-quality-auto')).toHaveAttribute('aria-checked', 'true')
  await expect(page.getByTestId('ab-quality-detail')).toContainText(/clear|清晰/i)
  await page.getByTestId('ab-quality-saver').tap()
  await expect(page.getByTestId('ab-quality-saver')).toHaveAttribute('aria-checked', 'true')
  await expect(screen).toHaveAttribute('data-tier', 'saver')
  await expect.poll(async () => (await received(page, 'quality')).at(-1)).toEqual({ type: 'quality', max_fps: 8, jpeg_quality: 40, pixel_ratio: 1 })
  await page.getByTestId('ab-menu-close').tap()

  // A fast connection does not take it back up.
  const sent = (await received(page, 'quality')).length
  await page.waitForTimeout(12_000)
  expect((await received(page, 'quality')).length).toBe(sent)
  await expect(screen).toHaveAttribute('data-tier', 'saver')
  expect((await shown(page)).pixels.width).toBe(412)

  // Kept on this device: the next time the screen starts as data saver.
  await openBrowser(page, true)
  await expect.poll(async () => (await received(page, 'hello'))[0]?.viewport).toMatchObject({ pixel_ratio: 1 })
  await expect.poll(async () => (await received(page, 'quality'))[0]).toEqual({ type: 'quality', max_fps: 8, jpeg_quality: 40, pixel_ratio: 1 })
  await expect(screen).toHaveAttribute('data-tier', 'saver')

  // Clear: the screen's density, whatever the connection.
  await page.getByTestId('ab-menu-button').tap()
  await page.getByTestId('ab-quality-clear').tap()
  await expect(screen).toHaveAttribute('data-tier', 'high')
  await page.getByTestId('ab-menu-close').tap()
  await slowDown(page, 15_000)
  await page.waitForTimeout(6000)
  await expect(screen).toHaveAttribute('data-tier', 'high')
  await expect.poll(async () => (await received(page, 'quality')).at(-1)).toEqual({ type: 'quality', max_fps: 15, jpeg_quality: 70, pixel_ratio: 2 })
})

test.describe('on a screen of 1.25', () => {
  test.use({ deviceScaleFactor: 1.25 })

  test('the clear tier asks for frames of CSS pixels', async ({ page }) => {
    await openBrowser(page)
    await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-tier', 'high')
    await expect.poll(async () => (await received(page, 'hello'))[0]?.viewport).toMatchObject({ pixel_ratio: 1 })
    await expect.poll(async () => (await received(page, 'quality'))[0]).toEqual({ type: 'quality', max_fps: 15, jpeg_quality: 70, pixel_ratio: 1 })
    const { width, pixels } = await shown(page)
    expect(pixels.width).toBe(width)
  })
})

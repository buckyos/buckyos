import { expect, test, type Page } from '@playwright/test'
import { TILE_HOLD_MS } from '../fixtures/desktop'

// An iPad in portrait: wide enough for the desktop layout, driven by touch.
test.use({
  viewport: { width: 820, height: 1180 },
  hasTouch: true,
  isMobile: true,
})

const nextFrame = (page: Page) =>
  page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))

/** Drags the Settings tile 150,90px in 30 steps; returns how far the tile's centre was from the pointer at each. */
async function dragSettings(page: Page, input: 'touch' | 'mouse') {
  const settings = page.getByTestId('desktop-app-settings')
  await expect(settings).toBeVisible()
  const box = (await settings.boundingBox())!
  const x = box.x + box.width / 2
  const y = box.y + box.height / 2
  const cdp = input === 'touch' ? await page.context().newCDPSession(page) : null
  if (cdp) {
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x, y, id: 1 }] })
    await page.waitForTimeout(TILE_HOLD_MS)
  } else {
    await page.mouse.move(x, y)
    await page.mouse.down()
  }
  const gaps: number[] = []
  for (let step = 1; step <= 30; step++) {
    const px = x + step * 5
    const py = y + step * 3
    if (cdp) await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: px, y: py, id: 1 }] })
    else await page.mouse.move(px, py)
    await nextFrame(page)
    const now = (await settings.boundingBox())!
    gaps.push(Math.hypot(px - (now.x + now.width / 2), py - (now.y + now.height / 2)))
  }
  if (cdp) {
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
    await cdp.detach()
  } else {
    await page.mouse.up()
  }
  return { before: box, gaps }
}

test('a tablet shows the desktop layout and a tap opens the app', async ({ page }) => {
  await page.goto('/?scenario=normal')
  await expect(page.locator('[data-form-factor]')).toHaveAttribute('data-form-factor', 'desktop')
  await page.getByTestId('desktop-app-settings').tap()
  await expect(page.getByText('Software Info')).toBeVisible()
})

for (const input of ['touch', 'mouse'] as const) {
  test(`a tile dragged by ${input} stays under the pointer and lands without opening`, async ({ page }) => {
    await page.goto('/?scenario=normal')
    const { before, gaps } = await dragSettings(page, input)
    expect(Math.max(...gaps)).toBeLessThan(2)
    const settings = page.getByTestId('desktop-app-settings')
    await expect.poll(async () => (await settings.boundingBox())!.x - before.x).toBeGreaterThan(100)
    await expect(page.getByText('Software Info')).toHaveCount(0)
    await expect(page.getByTestId('desktop-menu-show-in-files')).toHaveCount(0)
    // The press after a drag is an ordinary one again.
    if (input === 'touch') await settings.tap()
    else await settings.click()
    await expect(page.getByText('Software Info')).toBeVisible()
  })
}

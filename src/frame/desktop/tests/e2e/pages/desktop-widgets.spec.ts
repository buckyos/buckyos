import { expect, test, type Locator, type Page } from '@playwright/test'
import { MOCK_DESKTOP_FOLDER_KEY, seedDesktopFolder, TILE_HOLD_MS } from '../fixtures/desktop'

/** The mock Desktop folder (api/desktop_items_mock.ts): file name → entry. */
async function folderEntries(page: Page): Promise<Record<string, { kind: string; text?: string }>> {
  return page.evaluate((key) => JSON.parse(window.localStorage.getItem(key) ?? '{"entries":{}}').entries, MOCK_DESKTOP_FOLDER_KEY)
}

/** Where this device keeps an item (models/layout.ts `layoutStorageKey`): page index and cells. */
async function savedCells(page: Page, formFactor: 'desktop' | 'mobile', itemId: string) {
  return page.evaluate(([key, id]) => {
    const layout = JSON.parse(window.localStorage.getItem(key) ?? 'null') as { pages: Array<{ items: Array<{ id: string; x?: number; y?: number; w: number; h: number }> }> } | null
    for (const [pageIndex, page] of (layout?.pages ?? []).entries()) {
      const item = page.items.find((entry) => entry.id === id)
      if (item) return { page: pageIndex, x: item.x, y: item.y, w: item.w, h: item.h }
    }
    return null
  }, [`buckyos.layout.${formFactor}.v3`, itemId] as const)
}

const widgetText = (widget: string) => JSON.stringify({ widget })

/**
 * Waits for the desktop's menus and the Add dialog to finish closing: until then their backdrop takes the next press.
 * (A closing menu is hidden from the accessibility tree at once, so this counts its items.)
 */
async function settled(page: Page) {
  await expect(page.locator('[data-testid^="desktop-menu-"]')).toHaveCount(0)
  await expect(page.getByTestId('desktop-add-dialog')).toHaveCount(0)
}

/** A desktop with the Files shortcut and the clock only, so most of the first screen is empty. */
async function seedSparseDesktop(page: Page) {
  await seedDesktopFolder(page, { Files: 'files' }, { 'Clock.widget': { kind: 'file', size: 18, mtime: 1, text: widgetText('clock') } })
}

async function box(locator: Locator) {
  const value = await locator.boundingBox()
  if (!value) throw new Error('not shown')
  return value
}

/** A point on the shown screen's grid, `col`/`row` cells in (cell centre). */
async function cellPoint(page: Page, col: number, row: number) {
  const clock = await box(page.getByTestId('desktop-item-Clock.widget'))
  // The clock sits at 0,0 and is two cells wide and one row tall; the grid has a 2px gap.
  const colStep = (clock.width + 2) / 2
  const rowStep = clock.height + 2
  return { x: clock.x + col * colStep + colStep / 2, y: clock.y + row * rowStep + rowStep / 2 }
}

test.describe('desktop menu and widgets', () => {
  test.use({ viewport: { width: 1440, height: 900 } })

  test('the empty space has a menu: appearance settings, the other screens and Add…', async ({ page }) => {
    await page.goto('/?scenario=normal')
    const workspace = await box(page.getByTestId('desktop-workspace'))
    const empty = { x: workspace.x + workspace.width - 80, y: workspace.y + workspace.height / 2 }

    await page.mouse.click(empty.x, empty.y, { button: 'right' })
    await expect(page.getByTestId('desktop-menu-appearance')).toBeVisible()
    // The default desktop has two screens and shows the first.
    await expect(page.getByTestId('desktop-menu-previous-page')).toHaveAttribute('aria-disabled', 'true')
    await expect(page.getByTestId('desktop-menu-add')).not.toHaveAttribute('aria-disabled', 'true')
    // The item menu is not shown with it.
    await expect(page.getByTestId('desktop-menu-remove')).toHaveCount(0)
    await page.getByTestId('desktop-menu-next-page').click()
    await settled(page)
    await expect(page.getByTestId('desktop-app-preview')).toBeInViewport()
    await expect(page.getByTestId('desktop-app-messagehub')).not.toBeInViewport()

    await page.mouse.click(empty.x, empty.y, { button: 'right' })
    await expect(page.getByTestId('desktop-menu-next-page')).toHaveAttribute('aria-disabled', 'true')
    await page.getByTestId('desktop-menu-previous-page').click()
    await settled(page)
    await expect(page.getByTestId('desktop-app-messagehub')).toBeInViewport()

    await page.mouse.click(empty.x, empty.y, { button: 'right' })
    await page.getByTestId('desktop-menu-appearance').click()
    await expect(page.getByTestId('window-settings')).toBeVisible()
    await expect(page.getByTestId('settings-icon-size')).toBeVisible()
  })

  test('a right click on an item still opens its own menu', async ({ page }) => {
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-files').click({ button: 'right' })
    await expect(page.getByTestId('desktop-menu-remove')).toBeVisible()
    await expect(page.getByTestId('desktop-menu-appearance')).toHaveCount(0)
  })

  test('Add… lists widgets and apps, keeps what is on the desktop unchosen, and puts the choice where the menu was opened', async ({ page }) => {
    await seedSparseDesktop(page)
    await page.goto('/?scenario=normal')
    await expect(page.getByTestId('desktop-item-Clock.widget')).toBeVisible()

    const at = await cellPoint(page, 4, 2)
    await page.mouse.click(at.x, at.y, { button: 'right' })
    await page.getByTestId('desktop-menu-add').click()
    const dialog = page.getByTestId('desktop-add-dialog')
    await expect(dialog).toBeVisible()
    await expect(page.getByTestId('desktop-add-widget-clock')).toBeDisabled()
    await expect(page.getByTestId('desktop-add-widget-clock')).toContainText('On the desktop')
    await expect(page.getByTestId('desktop-add-widget-notepad')).toBeEnabled()
    await page.getByTestId('desktop-add-widget-system-info').click()
    await settled(page)

    // On the cell the menu was opened on, saved to the folder and to this device's layout.
    await expect(page.getByTestId('system-info-widget')).toBeVisible()
    expect(await savedCells(page, 'desktop', 'System Info.widget')).toEqual({ page: 0, x: 4, y: 2, w: 2, h: 3 })
    expect((await folderEntries(page))['System Info.widget']?.text).toContain('system-info')

    // System info now is on the desktop; a note can be added again and again.
    for (const [name, col] of [['Notepad.widget', 7], ['Notepad (2).widget', 6]] as const) {
      const note = await cellPoint(page, col, 0)
      await page.mouse.click(note.x, note.y, { button: 'right' })
      await page.getByTestId('desktop-menu-add').click()
      await expect(page.getByTestId('desktop-add-widget-system-info')).toBeDisabled()
      await page.getByTestId('desktop-add-widget-notepad').click()
      await settled(page)
      await expect(page.getByTestId(`desktop-item-${name}`)).toBeVisible()
    }
    // Two cells wide, the second note does not fit at 6,0 beside the first: it takes the nearest free spot.
    expect(await savedCells(page, 'desktop', 'Notepad.widget')).toEqual({ page: 0, x: 7, y: 0, w: 2, h: 2 })
    expect(await savedCells(page, 'desktop', 'Notepad (2).widget')).toEqual({ page: 0, x: 5, y: 0, w: 2, h: 2 })

    // Apps: the ones with a shortcut are shown but cannot be chosen.
    const appCell = await cellPoint(page, 10, 4)
    await page.mouse.click(appCell.x, appCell.y, { button: 'right' })
    await page.getByTestId('desktop-menu-add').click()
    await page.getByTestId('desktop-add-tab-apps').click()
    await expect(page.getByTestId('desktop-add-app-files')).toBeDisabled()
    await page.getByTestId('desktop-add-app-settings').click()
    await settled(page)
    await expect(page.getByTestId('desktop-app-settings')).toBeVisible()
    expect(Object.keys(await folderEntries(page))).toContain('Settings.lnk')

    await page.reload()
    await expect(page.getByTestId('desktop-app-settings')).toBeVisible()
    await expect(page.getByTestId('desktop-item-Notepad (2).widget')).toBeVisible()
    expect(await savedCells(page, 'desktop', 'System Info.widget')).toEqual({ page: 0, x: 4, y: 2, w: 2, h: 3 })
  })

  test('a widget is resized from its menu within its own limits, and the handles go when it loses focus', async ({ page }) => {
    await seedSparseDesktop(page)
    await page.goto('/?scenario=normal')
    const clock = page.getByTestId('desktop-item-Clock.widget')
    await expect(clock).toBeVisible()
    const start = await box(clock)
    const colStep = (start.width + 2) / 2
    const rowStep = start.height + 2
    const empty = await cellPoint(page, 8, 5)

    // Shortcuts have no Resize.
    await page.getByTestId('desktop-app-files').click({ button: 'right' })
    await expect(page.getByTestId('desktop-menu-remove')).toBeVisible()
    await expect(page.getByTestId('desktop-menu-resize')).toHaveCount(0)
    await page.keyboard.press('Escape')
    await settled(page)

    await clock.click({ button: 'right' })
    await page.getByTestId('desktop-menu-resize').click()
    await settled(page)
    const frame = page.getByTestId('desktop-resize-Clock.widget')
    await expect(frame).toBeVisible()
    await expect(frame.locator('.desktop-resize-handle')).toHaveCount(8)

    const drag = async (edge: string, dx: number, dy: number) => {
      const handle = await box(frame.getByTestId(`desktop-resize-handle-${edge}`))
      const x = handle.x + handle.width / 2
      const y = handle.y + handle.height / 2
      await page.mouse.move(x, y)
      await page.mouse.down()
      await page.mouse.move(x + dx / 2, y + dy / 2, { steps: 4 })
      await page.mouse.move(x + dx, y + dy, { steps: 4 })
      await page.mouse.up()
    }

    // The clock goes from 2 × 1 up to its largest, 4 × 2, and no further.
    await drag('se', colStep * 2, rowStep)
    await expect.poll(() => savedCells(page, 'desktop', 'Clock.widget')).toMatchObject({ x: 0, y: 0, w: 4, h: 2 })
    await expect.poll(async () => Math.abs((await box(clock)).width - (colStep * 4 - 2))).toBeLessThan(1.5)
    await drag('e', colStep * 3, 0)
    await expect.poll(() => savedCells(page, 'desktop', 'Clock.widget')).toMatchObject({ w: 4, h: 2 })

    // The left edge moves in, the right one stays: down to the smallest, 2 × 1.
    await drag('nw', colStep * 3, rowStep * 2)
    await expect.poll(() => savedCells(page, 'desktop', 'Clock.widget')).toMatchObject({ x: 2, y: 1, w: 2, h: 1 })

    // A press elsewhere takes the focus off it; the size stays.
    await page.mouse.click(empty.x, empty.y)
    await expect(frame).toHaveCount(0)

    await clock.click({ button: 'right' })
    await page.getByTestId('desktop-menu-resize').click()
    await settled(page)
    await expect(frame).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(frame).toHaveCount(0)

    await page.reload()
    await expect(page.getByTestId('desktop-item-Clock.widget')).toBeVisible()
    expect(await savedCells(page, 'desktop', 'Clock.widget')).toMatchObject({ x: 2, y: 1, w: 2, h: 1 })
  })

  test('a widget grown over shortcuts moves them aside', async ({ page }) => {
    await seedDesktopFolder(page, { Files: 'files', Settings: 'settings' }, { 'Clock.widget': { kind: 'file', size: 18, mtime: 1, text: widgetText('clock') } })
    await page.goto('/?scenario=normal')
    const clock = page.getByTestId('desktop-item-Clock.widget')
    await expect(clock).toBeVisible()
    const files = await savedCells(page, 'desktop', 'Files.lnk')
    expect(files).not.toBeNull()

    // Put Files right of the clock, then grow the clock over it.
    const right = await cellPoint(page, 2, 0)
    const filesTile = await box(page.getByTestId('desktop-app-files'))
    await page.mouse.move(filesTile.x + filesTile.width / 2, filesTile.y + filesTile.height / 2)
    await page.mouse.down()
    await page.mouse.move(right.x, right.y, { steps: 8 })
    await page.mouse.up()
    await expect.poll(() => savedCells(page, 'desktop', 'Files.lnk')).toMatchObject({ x: 2, y: 0 })

    await clock.click({ button: 'right' })
    await page.getByTestId('desktop-menu-resize').click()
    await settled(page)
    const handle = await box(page.getByTestId('desktop-resize-handle-e'))
    const colStep = ((await box(clock)).width + 2) / 2
    await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2)
    await page.mouse.down()
    await page.mouse.move(handle.x + handle.width / 2 + colStep, handle.y + handle.height / 2, { steps: 6 })
    await page.mouse.up()
    await expect.poll(() => savedCells(page, 'desktop', 'Clock.widget')).toMatchObject({ x: 0, y: 0, w: 3, h: 1 })
    const moved = await savedCells(page, 'desktop', 'Files.lnk')
    expect(moved && moved.x !== undefined && moved.y !== undefined && (moved.x >= 3 || moved.y >= 1)).toBe(true)
  })
})

test.describe('desktop menu on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true })

  /** One finger through CDP: down at `point`, held for `holdMs`, moved by each offset, then up. */
  async function touch(page: Page, point: { x: number; y: number }, holdMs: number, offsets: Array<{ x: number; y: number }> = []) {
    const cdp = await page.context().newCDPSession(page)
    try {
      await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ ...point, id: 1 }] })
      if (holdMs) await page.waitForTimeout(holdMs)
      for (const offset of offsets) {
        await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: point.x + offset.x, y: point.y + offset.y, id: 1 }] })
      }
      await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
    } finally {
      await cdp.detach()
    }
  }

  test('a hold on the empty space opens the menu; Add… puts an app under the finger and a widget resizes by touch', async ({ page }) => {
    await seedSparseDesktop(page)
    await page.goto('/?scenario=normal')
    await expect(page.getByTestId('desktop-item-Clock.widget')).toBeVisible()
    const clock = await box(page.getByTestId('desktop-item-Clock.widget'))
    const colStep = (clock.width + 2) / 2
    const rowStep = clock.height + 2
    // Bottom-right cell of the second row below the clock: empty.
    const point = { x: clock.x + colStep * 3.5, y: clock.y + rowStep * 3.5 }

    // A short touch is no menu, nor is a swipe.
    await touch(page, point, 0)
    await touch(page, point, 0, [{ x: -40, y: 0 }, { x: -80, y: 0 }])
    await expect(page.getByTestId('desktop-menu-add')).toHaveCount(0)

    await touch(page, point, TILE_HOLD_MS)
    await expect(page.getByTestId('desktop-menu-add')).toBeVisible()
    await page.getByTestId('desktop-menu-add').tap()
    await page.getByTestId('desktop-add-tab-apps').tap()
    await page.getByTestId('desktop-add-app-settings').tap()
    await settled(page)
    await expect(page.getByTestId('desktop-app-settings')).toBeVisible()
    expect(await savedCells(page, 'mobile', 'Settings.lnk')).toMatchObject({ page: 0, x: 3, y: 3 })

    // Hold the clock for its menu, then drag the bottom handle down a row.
    await touch(page, { x: clock.x + clock.width / 2, y: clock.y + clock.height / 2 }, TILE_HOLD_MS)
    await page.getByTestId('desktop-menu-resize').tap()
    await settled(page)
    const handle = await box(page.getByTestId('desktop-resize-handle-s'))
    const from = { x: handle.x + handle.width / 2, y: handle.y + handle.height / 2 }
    await touch(page, from, 0, [{ x: 0, y: rowStep / 3 }, { x: 0, y: rowStep * 2 / 3 }, { x: 0, y: rowStep }])
    await expect.poll(() => savedCells(page, 'mobile', 'Clock.widget')).toMatchObject({ x: 0, y: 0, w: 2, h: 2 })
    // The page did not turn under the drag.
    await expect(page.getByTestId('desktop-item-Clock.widget')).toBeInViewport()
  })
})

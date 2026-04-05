import { expect, test, type Locator } from '@playwright/test'

function boxesOverlap(
  left: { x: number; y: number; width: number; height: number } | null,
  right: { x: number; y: number; width: number; height: number } | null,
) {
  if (!left || !right) {
    return false
  }

  return !(
    left.x + left.width <= right.x ||
    right.x + right.width <= left.x ||
    left.y + left.height <= right.y ||
    right.y + right.height <= left.y
  )
}

function visibleLength(
  start: number,
  size: number,
  viewportExtent: number,
) {
  return Math.max(0, Math.min(start + size, viewportExtent) - Math.max(start, 0))
}

async function getScrollMetrics(locator: Locator) {
  return locator.evaluate((element) => ({
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight,
    scrollTop: element.scrollTop,
    distanceToBottom: element.scrollHeight - element.clientHeight - element.scrollTop,
  }))
}

test('desktop flow opens settings window and supports locale switch', async ({
  page,
}) => {
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') {
      consoleErrors.push(message.text())
    }
  })

  await page.goto('/?scenario=normal')

  await expect(page.getByRole('button', { name: 'BuckyOS' })).toBeVisible()
  await expect(page.getByTestId('desktop-item-widget-clock')).toBeVisible()
  await expect(page.getByTestId('notepad-preview-widget-notepad')).toBeVisible()
  await expect(page.getByTestId('notepad-editor-widget-notepad')).toHaveCount(0)
  await expect(page.getByTestId('notepad-save-widget-notepad')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Prototype Lab' })).toHaveCount(0)
  await expect(page.getByTestId('drag-settings')).toHaveCount(0)

  const settingsIcon = page.getByTestId('desktop-app-settings')
  const beforeDrag = await settingsIcon.boundingBox()
  await settingsIcon.hover()
  await page.mouse.down()
  await page.mouse.move((beforeDrag?.x ?? 0) + 120, (beforeDrag?.y ?? 0) + 120, {
    steps: 12,
  })
  await page.mouse.up()
  const afterDrag = await settingsIcon.boundingBox()
  expect(afterDrag?.x).not.toBe(beforeDrag?.x)

  await page.getByTestId('notepad-preview-widget-notepad').click()
  await expect(page.getByTestId('notepad-editor-widget-notepad')).toBeVisible()
  await expect(page.getByTestId('notepad-save-widget-notepad')).toBeVisible()
  await page.getByTestId('notepad-editor-widget-notepad').fill('Updated desktop note.')
  await page.getByTestId('notepad-save-widget-notepad').click()
  await expect(page.getByTestId('notepad-editor-widget-notepad')).toHaveCount(0)
  await expect(page.getByTestId('notepad-preview-widget-notepad')).toContainText(
    'Updated desktop note.',
  )

  const notepadWidget = page.getByTestId('desktop-item-widget-notepad')
  const widgetBeforeDrag = await notepadWidget.boundingBox()
  await notepadWidget.hover()
  await page.mouse.down()
  await page.mouse.move((widgetBeforeDrag?.x ?? 0) + 220, (widgetBeforeDrag?.y ?? 0) - 120, {
    steps: 14,
  })
  await page.mouse.up()
  const settingsAfterWidgetDrag = await page.getByTestId('desktop-app-settings').boundingBox()
  const filesAfterWidgetDrag = await page.getByTestId('desktop-app-files').boundingBox()
  expect(boxesOverlap(settingsAfterWidgetDrag, filesAfterWidgetDrag)).toBeFalsy()

  await page.getByTestId('desktop-app-settings').click()
  await expect(page.getByText('Software Info')).toBeVisible()
  const windowBeforeDrag = await page.getByTestId('window-settings').boundingBox()
  await page.getByTestId('window-drag-settings').hover()
  await page.mouse.down()
  await page.mouse.move((windowBeforeDrag?.x ?? 0) + 180, (windowBeforeDrag?.y ?? 0) + 96, {
    steps: 16,
  })
  await page.mouse.up()
  const windowAfterDrag = await page.getByTestId('window-settings').boundingBox()
  expect(windowAfterDrag?.x).not.toBe(windowBeforeDrag?.x)
  const windowBeforeResize = await page.getByTestId('window-settings').boundingBox()
  await page.getByTestId('window-resize-right-settings').hover()
  await page.mouse.down()
  await page.mouse.move(
    (windowBeforeResize?.x ?? 0) + (windowBeforeResize?.width ?? 0) + 120,
    (windowBeforeResize?.y ?? 0) + (windowBeforeResize?.height ?? 0) / 2,
    { steps: 14 },
  )
  await page.mouse.up()
  const windowAfterWidthResize = await page.getByTestId('window-settings').boundingBox()
  expect(windowAfterWidthResize?.width).toBeGreaterThan(windowBeforeResize?.width ?? 0)
  await expect(page.getByTestId('window-resize-bottom-left-settings')).toBeVisible()
  await page.getByTestId('window-resize-bottom-right-settings').hover()
  await page.mouse.down()
  await page.mouse.move(
    (windowAfterWidthResize?.x ?? 0) + (windowAfterWidthResize?.width ?? 0) + 90,
    (windowAfterWidthResize?.y ?? 0) + (windowAfterWidthResize?.height ?? 0) + 90,
    { steps: 14 },
  )
  await page.mouse.up()
  const windowAfterResize = await page.getByTestId('window-settings').boundingBox()
  expect(windowAfterResize?.height ?? 0).toBeGreaterThan(500)

  await page
    .getByTestId('window-settings')
    .getByRole('button', { name: 'Close' })
    .click()
  await expect(page.getByTestId('window-settings')).toHaveCount(0)

  await page.getByTestId('desktop-app-settings').click()
  await expect(page.getByTestId('window-settings')).toBeVisible()
  const reopenedWindow = await page.getByTestId('window-settings').boundingBox()
  expect(Math.abs((reopenedWindow?.x ?? 0) - (windowAfterResize?.x ?? 0))).toBeLessThan(2)
  expect(Math.abs((reopenedWindow?.y ?? 0) - (windowAfterResize?.y ?? 0))).toBeLessThan(2)
  expect(Math.abs((reopenedWindow?.width ?? 0) - (windowAfterResize?.width ?? 0))).toBeLessThan(2)
  expect(Math.abs((reopenedWindow?.height ?? 0) - (windowAfterResize?.height ?? 0))).toBeLessThan(2)

  await page.setViewportSize({ width: 960, height: 720 })
  await expect(page.getByTestId('window-settings')).toBeVisible()
  const hasOverflowAtDesktopMinimum = await page.evaluate(() => ({
    horizontal: document.documentElement.scrollWidth > window.innerWidth,
    vertical: document.documentElement.scrollHeight > window.innerHeight,
  }))
  expect(hasOverflowAtDesktopMinimum.horizontal).toBeFalsy()
  expect(hasOverflowAtDesktopMinimum.vertical).toBeFalsy()

  await page.setViewportSize({ width: 780, height: 560 })
  await expect(page.getByTestId('window-settings')).toBeVisible()
  const constrainedWindow = await page.getByTestId('window-settings').boundingBox()
  const closeButtonBox = await page
    .getByTestId('window-settings')
    .getByRole('button', { name: 'Close' })
    .boundingBox()
  expect((constrainedWindow?.x ?? 0) + (constrainedWindow?.width ?? 0)).toBeLessThanOrEqual(780)
  expect(constrainedWindow?.y ?? 0).toBeGreaterThanOrEqual(0)
  expect((closeButtonBox?.x ?? 0) + (closeButtonBox?.width ?? 0)).toBeLessThanOrEqual(780)
  expect(closeButtonBox?.y ?? 0).toBeLessThanOrEqual(560)
  const hasPageOverflow = await page.evaluate(() => ({
    horizontal: document.documentElement.scrollWidth > window.innerWidth,
    vertical: document.documentElement.scrollHeight > window.innerHeight,
  }))
  expect(hasPageOverflow.horizontal).toBeFalsy()
  expect(hasPageOverflow.vertical).toBeFalsy()

  await page.setViewportSize({ width: 1280, height: 720 })
  await expect(page.getByTestId('window-settings')).toBeVisible()
  await page
    .getByTestId('window-settings')
    .getByRole('button', { name: 'Appearance' })
    .click()
  await page.getByRole('combobox', { name: 'Language' }).selectOption('zh-CN')
  await page
    .getByTestId('window-settings')
    .getByRole('button', { name: '通用' })
    .click()
  await expect(page.getByText('软件信息')).toBeVisible()
  await page.getByLabel('关闭').click()

  expect(consoleErrors).toEqual([])
})

test('desktop layout restores AI Center launcher entry and opens panel content', async ({
  page,
}) => {
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') {
      consoleErrors.push(message.text())
    }
  })

  await page.addInitScript(() => {
    window.localStorage.setItem(
      'buckyos.layout.desktop.v1',
      JSON.stringify({
        version: 1,
        formFactor: 'desktop',
        deadZone: { top: 0, bottom: 8, left: 5, right: 5 },
        pages: [
          {
            id: 'desktop-page-1',
            items: [
              { id: 'widget-clock', type: 'widget', widgetType: 'clock', x: 0, y: 0, w: 2, h: 1, config: {} },
              { id: 'app-settings', type: 'app', appId: 'settings', x: 2, y: 0, w: 1, h: 1 },
              { id: 'app-files', type: 'app', appId: 'files', x: 3, y: 0, w: 1, h: 1 },
              { id: 'app-studio', type: 'app', appId: 'studio', x: 4, y: 0, w: 1, h: 1 },
              { id: 'app-market', type: 'app', appId: 'market', x: 5, y: 0, w: 1, h: 1 },
              { id: 'app-docs', type: 'app', appId: 'docs', x: 6, y: 0, w: 1, h: 1 },
              { id: 'app-demos', type: 'app', appId: 'demos', x: 7, y: 0, w: 1, h: 1 },
              { id: 'app-codeassistant', type: 'app', appId: 'codeassistant', x: 2, y: 1, w: 1, h: 1 },
              { id: 'app-messagehub', type: 'app', appId: 'messagehub', x: 3, y: 1, w: 1, h: 1 },
              {
                id: 'widget-notepad',
                type: 'widget',
                widgetType: 'notepad',
                x: 0,
                y: 1,
                w: 2,
                h: 2,
                config: {
                  content: 'Review drag semantics, dead zone behavior, and window polish.',
                },
              },
            ],
          },
          {
            id: 'desktop-page-2',
            items: [
              { id: 'app-diagnostics', type: 'app', appId: 'diagnostics', x: 0, y: 0, w: 1, h: 1 },
            ],
          },
        ],
      }),
    )
  })

  await page.goto('/?scenario=normal')

  await expect(page.getByTestId('desktop-app-ai-center')).toBeVisible()
  await page.getByTestId('desktop-app-ai-center').click()
  await expect(page.getByTestId('window-ai-center')).toBeVisible()
  await expect(page.getByText('AI Features Not Enabled')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Get Started' })).toBeVisible()

  expect(consoleErrors).toEqual([])
})

test('large window can move offscreen while keeping title bar reachable', async ({
  page,
}) => {
  await page.goto('/?scenario=normal')

  await page.getByTestId('desktop-app-settings').click()
  await expect(page.getByTestId('window-settings')).toBeVisible()

  const windowBeforeResize = await page.getByTestId('window-settings').boundingBox()
  await page.getByTestId('window-resize-right-settings').hover()
  await page.mouse.down()
  await page.mouse.move(
    (windowBeforeResize?.x ?? 0) + (windowBeforeResize?.width ?? 0) + 180,
    (windowBeforeResize?.y ?? 0) + 80,
    { steps: 16 },
  )
  await page.mouse.up()

  const windowAfterWidthResize = await page.getByTestId('window-settings').boundingBox()
  await page.getByTestId('window-resize-bottom-right-settings').hover()
  await page.mouse.down()
  await page.mouse.move(
    (windowAfterWidthResize?.x ?? 0) + (windowAfterWidthResize?.width ?? 0) + 160,
    (windowAfterWidthResize?.y ?? 0) + (windowAfterWidthResize?.height ?? 0) + 140,
    { steps: 16 },
  )
  await page.mouse.up()

  const desktopViewport = page.viewportSize()
  await page.getByTestId('window-drag-settings').hover()
  await page.mouse.down()
  await page.mouse.move(
    (desktopViewport?.width ?? 1280) + 640,
    (desktopViewport?.height ?? 720) + 420,
    { steps: 20 },
  )
  await page.mouse.up()

  const offscreenWindow = await page.getByTestId('window-settings').boundingBox()
  const offscreenTitleBar = await page.getByTestId('window-drag-settings').boundingBox()
  expect((offscreenWindow?.x ?? 0) + (offscreenWindow?.width ?? 0)).toBeGreaterThan(
    desktopViewport?.width ?? 1280,
  )
  expect((offscreenWindow?.y ?? 0) + (offscreenWindow?.height ?? 0)).toBeGreaterThan(
    desktopViewport?.height ?? 720,
  )
  expect(
    visibleLength(
      offscreenTitleBar?.x ?? 0,
      offscreenTitleBar?.width ?? 0,
      desktopViewport?.width ?? 1280,
    ),
  ).toBeGreaterThan(80)
  expect(offscreenTitleBar?.y ?? 0).toBeGreaterThanOrEqual(0)
  expect((offscreenTitleBar?.y ?? 0) + (offscreenTitleBar?.height ?? 0)).toBeLessThanOrEqual(
    desktopViewport?.height ?? 720,
  )
})

test('empty and error states render', async ({ page }) => {
  await page.goto('/?scenario=empty')
  await expect(page.getByText('Layout is empty')).toBeVisible()

  await page.goto('/?scenario=error')
  await expect(page.getByText('Mock data failed')).toBeVisible()
})

test('demos app renders common controls', async ({ page }) => {
  await page.goto('/?scenario=normal')

  await page.getByTestId('desktop-app-demos').click()
  await expect(page.getByTestId('window-demos')).toBeVisible()
  await expect(page.getByText('Control gallery', { exact: true })).toBeVisible()

  await page.getByRole('button', { name: 'Quick menu' }).click()
  await expect(page.getByRole('menuitem', { name: 'Pin to launcher' })).toBeVisible()
  await page.getByRole('menuitem', { name: 'Pin to launcher' }).click()

  await page.getByRole('textbox', { name: 'Search query' }).fill('State matrix')
  await expect(page.getByRole('textbox', { name: 'Search query' })).toHaveValue('State matrix')
  await page.getByRole('button', { name: 'Fullscreen modal' }).click()
  await expect(page.getByText('Fullscreen request: Denied')).toBeVisible()
  await page.getByRole('tab', { name: 'Status' }).click()
  await expect(page.getByText('Control coverage')).toBeVisible()
})

test('status tray tips opens from bell and closes on outside click', async ({ page }) => {
  await page.goto('/?scenario=normal')

  const tipsButton = page.getByTestId('status-tray-tips-button')
  await expect(tipsButton).toBeVisible()
  await tipsButton.click()

  const tipsPanel = page.getByTestId('status-tips-panel')
  await expect(tipsPanel).toBeVisible()
  await expect(page.getByTestId('status-tip-card-recent-shell-action')).toBeVisible()
  await expect(page.getByTestId('status-tip-card-mobile-touch-audit')).toBeVisible()

  const panelBox = await tipsPanel.boundingBox()
  const viewport = page.viewportSize()
  expect(panelBox).not.toBeNull()
  expect(viewport).not.toBeNull()
  expect((panelBox?.x ?? 0) + (panelBox?.width ?? 0)).toBeLessThanOrEqual(
    (viewport?.width ?? 0) - 8,
  )
  expect(panelBox?.x ?? 0).toBeGreaterThanOrEqual(0)

  await page.mouse.click(24, (viewport?.height ?? 0) - 24)
  await expect(tipsPanel).toHaveCount(0)
})

test('window modal only blocks its owner window', async ({ page }) => {
  await page.goto('/?scenario=normal')

  await page.getByTestId('desktop-app-settings').click()
  await expect(page.getByTestId('window-settings')).toBeVisible()

  const settingsBeforeDrag = await page.getByTestId('window-settings').boundingBox()
  await page.getByTestId('window-drag-settings').hover()
  await page.mouse.down()
  await page.mouse.move(
    1180,
    (settingsBeforeDrag?.y ?? 0) + 90,
    { steps: 18 },
  )
  await page.mouse.up()

  await page.getByTestId('desktop-app-demos').click()
  await expect(page.getByTestId('window-demos')).toBeVisible()
  await page.getByRole('button', { name: 'Window modal' }).first().click()
  await expect(page.getByRole('dialog', { name: 'Scoped window modal' })).toBeVisible()

  await page.getByRole('combobox', { name: 'Language' }).selectOption('ja')
  await expect(page.getByRole('combobox', { name: 'Language' })).toHaveValue('ja')

  await expect(page.getByRole('dialog', { name: 'Scoped window modal' })).toBeVisible()
  await page
    .getByTestId('window-settings')
    .getByRole('button', { name: 'Close' })
    .click()
  await expect(page.getByTestId('window-settings')).toHaveCount(0)
  await expect(page.getByRole('dialog', { name: 'Scoped window modal' })).toBeVisible()
  await page.getByRole('button', { name: 'Apply change' }).click()
  await expect(page.getByRole('dialog', { name: 'Scoped window modal' })).toHaveCount(0)
  await expect(page.getByRole('textbox', { name: 'Owner' })).toHaveValue('Window modal owner')
})

test('codeassistant history does not jump back to bottom while scrolling older messages', async ({
  page,
}) => {
  await page.goto('/?scenario=normal')

  await page.getByTestId('desktop-app-codeassistant').click()
  const historyPane = page.locator('[data-testid="window-codeassistant"] .shell-scrollbar').first()
  await expect(historyPane).toBeVisible()

  await expect.poll(async () => {
    const { scrollHeight, clientHeight, distanceToBottom } = await getScrollMetrics(historyPane)
    return scrollHeight > clientHeight ? distanceToBottom : Number.POSITIVE_INFINITY
  }).toBeLessThanOrEqual(24)

  await historyPane.hover()

  let scrolledMetrics = await getScrollMetrics(historyPane)
  for (let attempt = 0; attempt < 4; attempt += 1) {
    await page.mouse.wheel(0, -1200)
    await page.waitForTimeout(60)
    scrolledMetrics = await getScrollMetrics(historyPane)
    if (scrolledMetrics.distanceToBottom > 600) {
      break
    }
  }

  expect(scrolledMetrics.distanceToBottom).toBeGreaterThan(600)

  await page.waitForTimeout(900)
  const settledMetrics = await getScrollMetrics(historyPane)
  expect(settledMetrics.distanceToBottom).toBeGreaterThan(600)
})

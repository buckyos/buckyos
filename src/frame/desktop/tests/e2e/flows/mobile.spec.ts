import { expect, test, type Locator, type Page } from '@playwright/test'
import { openFromAllApps } from '../fixtures/desktop'

test.use({
  viewport: { width: 375, height: 812 },
  hasTouch: true,
  isMobile: true,
})

test('mobile viewport opens in-place app with system title bar', async ({
  page,
}) => {
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') {
      consoleErrors.push(message.text())
    }
  })

  await page.goto('/?scenario=normal')

  await expect(page.getByTestId('drag-settings')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Settings' })).toBeVisible()
  await expect(page.getByLabel('Status bar').locator('.shell-pill:visible')).toHaveCount(1)
  await expect(page.getByText('Secure session')).toHaveCount(0)

  const filesTile = page.getByTestId('desktop-item-Files.lnk')
  const beforeDrag = await filesTile.boundingBox()
  await filesTile.hover()
  await page.mouse.down()
  await page.mouse.move((beforeDrag?.x ?? 0) - 56, (beforeDrag?.y ?? 0) + 108, {
    steps: 14,
  })
  await page.mouse.up()
  const afterDrag = await filesTile.boundingBox()
  expect(afterDrag?.x !== beforeDrag?.x || afterDrag?.y !== beforeDrag?.y).toBeTruthy()

  await page.reload()
  const settingsButton = page.getByRole('button', { name: 'Settings' })
  await settingsButton.tap()
  await expect(page.getByPlaceholder('Search settings')).toBeVisible()
  await expect(page.getByRole('button', { name: 'General' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'App menu' })).toBeVisible()
  const minimizeButton = page.getByRole('button', { name: 'Minimize' })
  await expect(minimizeButton).toBeVisible()
  await minimizeButton.tap()
  await expect(page.getByPlaceholder('Search settings')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Settings' })).toBeVisible()

  expect(consoleErrors).toEqual([])
})

test('mobile launcher shows AI Center and opens panel content', async ({
  page,
}) => {
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') {
      consoleErrors.push(message.text())
    }
  })

  await page.goto('/?scenario=normal')

  const aiCenterButton = page.getByRole('button', { name: 'AI Center' })
  await expect(aiCenterButton).toBeVisible()

  await aiCenterButton.tap()

  await expect(page.getByText('AI Features Not Enabled')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Get Started' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'App menu' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Minimize' })).toBeVisible()
  await expect(page.getByText('Manage AI providers, models, usage, and routing.')).toBeVisible()

  expect(consoleErrors).toEqual([])
})

async function touchGesture(
  page: Page,
  tile: Locator,
  offsets: Array<{ x: number; y: number }>,
) {
  await tile.tap({ trial: true })
  const box = await tile.boundingBox()
  expect(box).not.toBeNull()
  const x = box!.x + box!.width / 2
  const y = box!.y + box!.height / 2
  const cdp = await page.context().newCDPSession(page)
  try {
    await cdp.send('Input.dispatchTouchEvent', {
      type: 'touchStart',
      touchPoints: [{ x, y, id: 1 }],
    })
    for (const offset of offsets) {
      await cdp.send('Input.dispatchTouchEvent', {
        type: 'touchMove',
        touchPoints: [{ x: x + offset.x, y: y + offset.y, id: 1 }],
      })
    }
    await cdp.send('Input.dispatchTouchEvent', {
      type: 'touchEnd',
      touchPoints: [],
    })
  } finally {
    await cdp.detach()
  }
}

for (const offset of [{ x: 5, y: 0 }, { x: 7, y: 6 }]) {
  test(`mobile launcher opens after a short touch with ${offset.x},${offset.y}px drift`, async ({ page }) => {
    await page.goto('/?scenario=normal')
    await touchGesture(page, page.getByRole('button', { name: 'Settings', exact: true }), [offset])
    await expect(page.getByPlaceholder('Search settings')).toBeVisible()
  })
}

test('mobile launcher touch drag moves an icon without opening the app', async ({ page }) => {
  await page.goto('/?scenario=normal')
  const settings = page.getByRole('button', { name: 'Settings', exact: true })
  await settings.tap({ trial: true })
  const before = await settings.boundingBox()
  expect(before).not.toBeNull()
  await touchGesture(page, settings, [
    { x: 0, y: -30 },
    { x: 0, y: -80 },
    { x: 0, y: -140 },
  ])
  await expect.poll(async () => {
    const after = await settings.boundingBox()
    return Math.abs((after?.y ?? before!.y) - before!.y)
  }).toBeGreaterThan(60)
  await expect(page.getByRole('button', { name: 'Minimize' })).toHaveCount(0)
  await settings.tap()
  await expect(page.getByPlaceholder('Search settings')).toBeVisible()
})

test('mobile launcher does not open an app after dragging away and back', async ({ page }) => {
  await page.goto('/?scenario=normal')
  const settings = page.getByRole('button', { name: 'Settings', exact: true })
  await touchGesture(page, settings, [
    { x: 0, y: -30 },
    { x: 0, y: -80 },
    { x: 0, y: -30 },
    { x: 0, y: 0 },
  ])
  await expect(page.getByRole('button', { name: 'Minimize' })).toHaveCount(0)
  await expect(settings).toBeVisible()
  await settings.tap()
  await expect(page.getByPlaceholder('Search settings')).toBeVisible()
})

for (const input of ['mouse', 'Enter', 'Space']) {
  test(`mobile launcher opens with ${input}`, async ({ page }) => {
    await page.goto('/?scenario=normal')
    const settings = page.getByRole('button', { name: 'Settings', exact: true })
    if (input === 'mouse') {
      await settings.click()
    } else {
      await settings.press(input)
    }
    await expect(page.getByPlaceholder('Search settings')).toBeVisible()
  })
}

test('mobile demos exposes dialog trigger and opens centered dialog', async ({
  page,
}) => {
  await page.goto('/?scenario=normal')
  await expect(page.getByTestId('mobile-taskbar')).toBeAttached()
  // Demos has no desktop shortcut; it is under All apps.
  await openFromAllApps(page, 'demos')

  await expect(page.getByText('Control gallery', { exact: true })).toBeVisible()
  const trigger = page.getByRole('button', { name: 'Window modal' }).last()
  await expect(trigger).toBeVisible()
  await trigger.tap()
  const dialog = page.getByRole('dialog', { name: 'Scoped window modal' })
  await expect(dialog).toBeVisible()

  const viewport = page.viewportSize()
  const dialogBox = await dialog.boundingBox()
  expect(viewport).not.toBeNull()
  expect(dialogBox).not.toBeNull()

  const viewportCenterX = (viewport?.width ?? 0) / 2
  const viewportCenterY = (viewport?.height ?? 0) / 2
  const dialogCenterX = (dialogBox?.x ?? 0) + (dialogBox?.width ?? 0) / 2
  const dialogCenterY = (dialogBox?.y ?? 0) + (dialogBox?.height ?? 0) / 2

  expect(Math.abs(dialogCenterX - viewportCenterX)).toBeLessThan(24)
  expect(Math.abs(dialogCenterY - viewportCenterY)).toBeLessThan(40)
})

test('mobile status tray panels stay within viewport and dismiss on blur', async ({
  page,
}) => {
  await page.goto('/?scenario=normal')

  const bellButton = page.getByTestId('status-tray-notifications-button')
  await expect(bellButton).toBeVisible()
  await bellButton.click({ force: true })

  const panel = page.getByTestId('status-tray-notifications-panel')
  await expect(panel).toBeVisible()
  await expect(panel.getByTestId('tray-notify-row').first()).toBeVisible()

  const panelBox = await panel.boundingBox()
  const viewport = page.viewportSize()
  expect(panelBox).not.toBeNull()
  expect(viewport).not.toBeNull()
  expect(panelBox?.x ?? 0).toBeGreaterThanOrEqual(0)
  expect((panelBox?.x ?? 0) + (panelBox?.width ?? 0)).toBeLessThanOrEqual(
    viewport?.width ?? 0,
  )
  expect((panelBox?.y ?? 0) + (panelBox?.height ?? 0)).toBeLessThanOrEqual(
    viewport?.height ?? 0,
  )

  await page.locator('body').click({
    force: true,
    position: { x: 12, y: viewport ? viewport.height - 12 : 780 },
  })
  await expect(panel).toHaveCount(0)
})

test('mobile App Service opens without redundant back button on detail page', async ({
  page,
}) => {
  const consoleErrors: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'error') {
      consoleErrors.push(message.text())
    }
  })

  await page.goto('/?scenario=normal')

  // Open App Service
  const appServiceButton = page.getByRole('button', { name: 'App Service' })
  await expect(appServiceButton).toBeVisible()

  await appServiceButton.tap()

  // Home page should be visible with app cards
  await expect(page.getByRole('heading', { name: 'App Service' })).toBeVisible()
  const appCard = page.getByTestId('app-service-card-nostr-relay.buckyos.bns.did@alice')
  await expect(appCard).toBeVisible()
  await appCard.tap()
  await expect(page.getByTestId('app-service-runtime-status')).toBeVisible()

  // The in-page "← Back" text button should NOT be present on mobile
  // (the title bar already provides back navigation via its own back arrow)
  // The in-page back button has text content "Back" with an ArrowLeft icon
  await expect(page.locator('button:has(svg)', { hasText: 'Back' })).toHaveCount(0)

  expect(consoleErrors).toEqual([])
})

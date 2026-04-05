import { expect, test } from '@playwright/test'

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

  const filesTile = page.getByTestId('desktop-item-app-files-mobile')
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
  const settingsBox = await settingsButton.boundingBox()
  expect(settingsBox).not.toBeNull()

  const startX = (settingsBox?.x ?? 0) + (settingsBox?.width ?? 0) / 2
  const startY = (settingsBox?.y ?? 0) + (settingsBox?.height ?? 0) / 2

  await settingsButton.dispatchEvent('pointerdown', {
    bubbles: true,
    clientX: startX,
    clientY: startY,
    pointerId: 1,
    pointerType: 'touch',
  })
  await page.locator('body').dispatchEvent('pointerup', {
    bubbles: true,
    clientX: startX + 7,
    clientY: startY + 6,
    pointerId: 1,
    pointerType: 'touch',
  })
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

  const aiCenterBox = await aiCenterButton.boundingBox()
  expect(aiCenterBox).not.toBeNull()

  const startX = (aiCenterBox?.x ?? 0) + (aiCenterBox?.width ?? 0) / 2
  const startY = (aiCenterBox?.y ?? 0) + (aiCenterBox?.height ?? 0) / 2

  await aiCenterButton.dispatchEvent('pointerdown', {
    bubbles: true,
    clientX: startX,
    clientY: startY,
    pointerId: 3,
    pointerType: 'touch',
  })
  await page.locator('body').dispatchEvent('pointerup', {
    bubbles: true,
    clientX: startX + 6,
    clientY: startY + 5,
    pointerId: 3,
    pointerType: 'touch',
  })

  await expect(page.getByText('AI Features Not Enabled')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Get Started' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'App menu' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Minimize' })).toBeVisible()
  await expect(page.getByText('Manage AI providers, models, usage, and routing.')).toBeVisible()

  expect(consoleErrors).toEqual([])
})

test('mobile demos exposes dialog trigger and opens centered dialog', async ({
  page,
}) => {
  await page.goto('/?scenario=normal')

  const demosButton = page.getByRole('button', { name: 'Demos' })
  const demosBox = await demosButton.boundingBox()
  expect(demosBox).not.toBeNull()

  const startX = (demosBox?.x ?? 0) + (demosBox?.width ?? 0) / 2
  const startY = (demosBox?.y ?? 0) + (demosBox?.height ?? 0) / 2

  await demosButton.dispatchEvent('pointerdown', {
    bubbles: true,
    clientX: startX,
    clientY: startY,
    pointerId: 2,
    pointerType: 'touch',
  })
  await page.locator('body').dispatchEvent('pointerup', {
    bubbles: true,
    clientX: startX + 6,
    clientY: startY + 5,
    pointerId: 2,
    pointerType: 'touch',
  })

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

test('mobile status tray tips stays within viewport and dismisses on blur', async ({
  page,
}) => {
  await page.goto('/?scenario=normal')

  const tipsButton = page.getByTestId('status-tray-tips-button')
  await expect(tipsButton).toBeVisible()
  await tipsButton.click({ force: true })

  const tipsPanel = page.getByTestId('status-tips-panel')
  await expect(tipsPanel).toBeVisible()
  await expect(page.getByTestId('status-tip-card-diagnostics-export')).toBeVisible()

  const panelBox = await tipsPanel.boundingBox()
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
  await expect(tipsPanel).toHaveCount(0)
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

  const box = await appServiceButton.boundingBox()
  expect(box).not.toBeNull()

  const startX = (box?.x ?? 0) + (box?.width ?? 0) / 2
  const startY = (box?.y ?? 0) + (box?.height ?? 0) / 2

  await appServiceButton.dispatchEvent('pointerdown', {
    bubbles: true,
    clientX: startX,
    clientY: startY,
    pointerId: 5,
    pointerType: 'touch',
  })
  await page.locator('body').dispatchEvent('pointerup', {
    bubbles: true,
    clientX: startX + 6,
    clientY: startY + 5,
    pointerId: 5,
    pointerType: 'touch',
  })

  // Home page should be visible with app cards
  await expect(page.getByRole('heading', { name: 'App Service' })).toBeVisible()
  await expect(page.getByText('Nostr Relay')).toBeVisible()

  // Click on Gitea (error state) to open detail page
  const giteaCard = page.getByRole('button', { name: /Gitea/ }).first()
  await giteaCard.click()

  // Detail page should show Gitea info
  await expect(page.getByText('Self-hosted Git service')).toBeVisible()

  // The in-page "← Back" text button should NOT be present on mobile
  // (the title bar already provides back navigation via its own back arrow)
  // The in-page back button has text content "Back" with an ArrowLeft icon
  await expect(page.locator('button:has(svg)', { hasText: 'Back' })).toHaveCount(0)

  expect(consoleErrors).toEqual([])
})

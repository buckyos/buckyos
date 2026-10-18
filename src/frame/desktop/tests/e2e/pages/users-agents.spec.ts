import { expect, test } from '@playwright/test'

test.describe('Users & Agents app panel', () => {
  test('desktop: internal entity manager renders and validates local-user form', async ({ page }) => {
    const consoleErrors: string[] = []
    let currentAction = 'open page'
    page.on('console', (message) => {
      if (message.type() === 'error') consoleErrors.push(`${currentAction}: ${message.text()}`)
    })

    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/?scenario=normal')

    await page.getByTestId('desktop-app-users-agents').click()
    const win = page.getByTestId('window-users-agents')
    await expect(win).toBeVisible()

    await expect(win.getByText('Internal Entities')).toBeVisible()
    await expect(win.getByLabel('Add User')).toBeVisible()
    await expect(win.getByLabel('Add Agent')).toBeVisible()
    await expect(win.getByText('BuckyOS Assistant')).toBeVisible()
    await expect(win.getByText('Carol')).toBeVisible()
    await expect(win.getByText('Collections', { exact: true })).toHaveCount(0)

    await win.getByLabel('Search or filter').click()
    await win.getByPlaceholder('Search name, DID, role, tag, status...').fill('Carol')
    await expect(win.getByText('Carol')).toBeVisible()
    await expect(win.getByText('Bob')).toHaveCount(0)

    await win.getByText('Carol').click()
    await expect(win.getByText('Pending DID Confirmation')).toBeVisible()
    await expect(win.getByText('Target user must confirm binded_zone_list update')).toBeVisible()
    await expect(win.locator('button[aria-current="page"]')).toHaveCount(1)

    await win.getByPlaceholder('Search name, DID, role, tag, status...').fill('assistant')
    await win.getByText('BuckyOS Assistant').click()
    const detail = win.getByTestId('agent-detail')
    await expect(detail.getByTestId('agent-install-status')).toContainText('Running')
    await expect(detail.getByTestId('agent-profile')).toContainText('assistant')
    await expect(detail.getByTestId('agent-channels')).toContainText('@bucky_bot')
    await expect(detail.getByRole('switch', { name: 'Allow this Agent to join group chats' })).not.toBeChecked()
    await expect(detail.getByRole('switch', { name: 'Allow other users to use this Agent' })).toBeDisabled()

    currentAction = 'open Add Agent'
    await win.getByLabel('Add Agent').click()
    const wizard = page.getByTestId('window-agent-setup')
    await expect(wizard.getByText('Create Agent', { exact: true }).last()).toBeVisible()
    await expect(wizard.getByTestId('agent-setup-name')).toHaveValue('')
    await wizard.getByRole('button', { name: 'Close' }).first().click()
    await win.click({ position: { x: 400, y: 20 } })

    currentAction = 'open Add User'
    await win.getByLabel('Add User').click()
    await expect(win.getByText('New Local User')).toBeVisible()
    await expect(win.getByText('Primary DID invite')).toHaveCount(0)
    await expect(win.getByText('Available apps')).toHaveCount(0)
    await expect(win.getByText('Storage quota')).toHaveCount(0)
    await win.getByLabel('Local username').fill('Maria.Local')
    await win.getByLabel('Display name').fill('Maria')
    await win.getByLabel('Initial password').fill('local-password-1')
    await win.getByLabel('Confirm password').fill('different-password')
    currentAction = 'validate password mismatch'
    await win.getByRole('button', { name: 'Next' }).click()
    await expect(win.getByText('Passwords do not match.')).toBeVisible()
    await win.getByLabel('Confirm password').fill('local-password-1')
    currentAction = 'open review'
    await win.getByRole('button', { name: 'Next' }).click()
    await expect(win.getByText('maria.local')).toBeVisible()
    await expect(win.getByText('No apps are installed or promised')).toBeVisible()
    await expect(win.getByRole('button', { name: 'Create User', exact: true })).toBeVisible()
    currentAction = 'close wizard'
    await win.getByLabel('Close new user wizard').click()

    expect(consoleErrors).toEqual([])
  })
})

test.describe('Users & Agents mobile panel', () => {
  test.use({
    viewport: { width: 375, height: 812 },
    hasTouch: true,
    isMobile: true,
  })

  test('mobile: internal entity search and detail layers stay within viewport', async ({ page }) => {
    const consoleErrors: string[] = []
    page.on('console', (message) => {
      if (message.type() === 'error') consoleErrors.push(message.text())
    })

    await page.goto('/?scenario=normal')

    const usersAgentsButton = page.getByRole('button', { name: 'Users & Agents' })
    await usersAgentsButton.scrollIntoViewIfNeeded()
    const box = await usersAgentsButton.boundingBox()
    expect(box).not.toBeNull()

    const startX = (box?.x ?? 0) + (box?.width ?? 0) / 2
    const startY = (box?.y ?? 0) + (box?.height ?? 0) / 2
    await usersAgentsButton.dispatchEvent('pointerdown', {
      bubbles: true,
      clientX: startX,
      clientY: startY,
      pointerId: 7,
      pointerType: 'touch',
    })
    await page.locator('body').dispatchEvent('pointerup', {
      bubbles: true,
      clientX: startX + 6,
      clientY: startY + 6,
      pointerId: 7,
      pointerType: 'touch',
    })

    await expect(page.getByText('Internal Entities', { exact: true })).toBeVisible()
    await expect(page.getByText('Collections', { exact: true })).toHaveCount(0)
    await page.getByLabel('Search or filter').click()
    await page.getByPlaceholder('Search name, DID, role, tag, status...').fill('Carol')
    await page.getByRole('button', { name: /Carol/ }).click()
    await expect(page.getByText('Pending DID Confirmation')).toBeVisible()

    await page.getByRole('button', { name: 'Back', exact: true }).click()
    await page.getByLabel('Search or filter').click()
    await page.getByPlaceholder('Search name, DID, role, tag, status...').fill('assistant')
    await page.getByRole('button', { name: /BuckyOS Assistant/ }).click()
    await page.getByRole('button', { name: 'View Agent sessions', exact: true }).click()
    const hub = page.getByTestId('owner-banner')
    await expect(hub).toContainText('assistant.alice')
    await page.getByRole('button', { name: 'Session details', exact: true }).last().click()
    await expect(page.getByTestId('session-details').getByRole('button', { name: 'Manage session', exact: true })).toBeDisabled()
    await page.screenshot({ path: 'test-results/messagehub-mobile-embedded-observer.png' })
    await page.getByRole('button', { name: 'Return to my messages', exact: true }).click()
    await page.getByRole('button', { name: 'Entity details: Alice Chen' }).click()
    await page.getByLabel('Allow manual sessions with this entity').selectOption('allow')
    await page.getByRole('button', { name: 'Save', exact: true }).click()
    await page.getByRole('button', { name: 'Close', exact: true }).last().click()
    await page.getByRole('button', { name: 'New Session', exact: true }).last().click()
    const dialog = page.getByRole('dialog')
    await dialog.getByRole('combobox', { name: 'Connection', exact: true }).selectOption('native')
    await dialog.getByLabel('Title (optional)').fill('Mobile window session')
    await page.screenshot({ path: 'test-results/messagehub-mobile-embedded-create.png' })
    await dialog.getByRole('button', { name: 'Create', exact: true }).click()
    await expect(dialog).toHaveCount(0)
    await page.getByRole('button', { name: 'Session details', exact: true }).last().click()
    await page.getByTestId('session-details').getByRole('button', { name: 'Manage session', exact: true }).click()
    await page.screenshot({ path: 'test-results/messagehub-mobile-embedded-manage.png' })
    await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click()
    await expect(page.getByRole('dialog')).toHaveCount(0)

    const overflow = await page.evaluate(() => ({
      horizontal: document.documentElement.scrollWidth > window.innerWidth,
      vertical: document.documentElement.scrollHeight > window.innerHeight,
    }))
    expect(overflow.horizontal).toBeFalsy()
    expect(overflow.vertical).toBeFalsy()
    expect(consoleErrors).toEqual([])
  })
})

test('Agent detail opens an explicit read-only owner view in a desktop window', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-users-agents').click()
  const users = page.getByTestId('window-users-agents')
  await users.getByText('BuckyOS Assistant', { exact: true }).click()
  await expect(users.getByRole('button', { name: 'Chat with Agent', exact: true })).toBeVisible()
  await users.getByRole('button', { name: 'View Agent sessions', exact: true }).click()
  const hub = page.getByTestId('window-messagehub')
  await expect(hub.getByTestId('owner-banner')).toContainText('assistant.alice')
  await expect(hub.getByTestId('owner-banner')).toContainText('Read only')
  await hub.getByRole('button', { name: 'Session details', exact: true }).last().click()
  await expect(hub.getByTestId('session-details').getByRole('button', { name: 'Manage session', exact: true })).toBeDisabled()
  await page.screenshot({ path: 'test-results/messagehub-desktop-agent-observer.png' })
  await hub.getByRole('button', { name: 'Return to my messages', exact: true }).click()
  await expect(hub.getByTestId('owner-banner')).toHaveCount(0)
  await expect(hub.locator('textarea')).toBeVisible()
  await expect(page).toHaveURL(/scenario=normal/)
  await hub.getByRole('button', { name: 'Entity details: Alice Chen' }).click()
  await hub.getByLabel('Allow manual sessions with this entity').selectOption('allow')
  await hub.getByRole('button', { name: 'Save', exact: true }).click()
  await hub.getByRole('button', { name: 'Close', exact: true }).last().click()
  await hub.getByRole('button', { name: 'New Session', exact: true }).last().click()
  const dialog = hub.getByRole('dialog')
  await dialog.getByRole('combobox', { name: 'Connection', exact: true }).selectOption('native')
  await dialog.getByLabel('Title (optional)').fill('Desktop window session')
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await expect(hub.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '0')
  await hub.getByRole('button', { name: 'Session details', exact: true }).last().click()
  await hub.getByTestId('session-details').getByRole('button', { name: 'Manage session', exact: true }).click()
  await page.screenshot({ path: 'test-results/messagehub-desktop-manage.png' })
  await hub.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click()
  await expect(hub.getByRole('dialog')).toHaveCount(0)
})

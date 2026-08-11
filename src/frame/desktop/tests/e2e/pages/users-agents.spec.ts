import { expect, test } from '@playwright/test'

function placeUsersAgentsOnFirstPage() {
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
            {
              id: 'app-users-agents',
              type: 'app',
              appId: 'users-agents',
              x: 0,
              y: 0,
              w: 1,
              h: 1,
            },
          ],
        },
      ],
    }),
  )
}

function placeMyNetworkOnFirstPage() {
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
            {
              id: 'app-my-network',
              type: 'app',
              appId: 'my-network',
              x: 0,
              y: 0,
              w: 1,
              h: 1,
            },
          ],
        },
      ],
    }),
  )
}

test.describe('Users & Agents app panel', () => {
  test('desktop: internal entity manager renders and mutates mock state', async ({ page }) => {
    const consoleErrors: string[] = []
    page.on('console', (message) => {
      if (message.type() === 'error') consoleErrors.push(message.text())
    })

    await page.setViewportSize({ width: 1440, height: 900 })
    await page.addInitScript(placeUsersAgentsOnFirstPage)
    await page.goto('/?scenario=normal')

    await page.getByTestId('desktop-app-users-agents').click()
    const win = page.getByTestId('window-users-agents')
    await expect(win).toBeVisible()

    await expect(win.getByText('Internal Entities')).toBeVisible()
    await expect(win.getByLabel('Add User')).toBeVisible()
    await expect(win.getByLabel('Add Agent')).toBeVisible()
    await expect(win.getByText('BuckyOS Assistant')).toBeVisible()
    await expect(win.getByText('Carol')).toBeVisible()
    await expect(win.getByText('Family Space')).toBeVisible()
    await expect(win.getByText('Collections', { exact: true })).toHaveCount(0)

    await win.getByLabel('Search or filter').click()
    await win.getByPlaceholder('Search name, DID, role, tag, status...').fill('Carol')
    await expect(win.getByText('Carol')).toBeVisible()
    await expect(win.getByText('Bob')).toHaveCount(0)

    await win.getByText('Carol').click()
    await expect(win.getByText('Pending DID Confirmation')).toBeVisible()
    await expect(win.getByText('Target user must confirm binded_zone_list update')).toBeVisible()

    await win.getByPlaceholder('Search name, DID, role, tag, status...').fill('assistant')
    await win.getByText('BuckyOS Assistant').click()
    await expect(win.getByText('Runtime & Work')).toBeVisible()
    await expect(win.getByText('Running Tasks')).toBeVisible()
    await expect(win.getByText('Queued Tasks')).toBeVisible()
    await expect(win.getByText('Social Accounts')).toBeVisible()

    await win.getByLabel('Add Agent').click()
    await expect(page.getByText('Agent creation is coming soon.')).toBeVisible()

    await win.getByLabel('Add User').click()
    await expect(win.getByText('New Zone User')).toBeVisible()
    await win.getByRole('button', { name: 'Next' }).click()
    await win.getByRole('button', { name: 'Next' }).click()
    await win.getByLabel('BNS name or DID').fill('maria')
    await win.getByLabel('Display name').fill('Maria')
    await win.getByRole('button', { name: 'Next' }).click()
    await win.getByRole('button', { name: 'Next' }).click()
    const createInvitation = win.getByRole('button', { name: 'Create Invitation', exact: true })
    await expect(createInvitation).toBeVisible()
    await createInvitation.evaluate((button) => (button as HTMLButtonElement).click())
    await expect(win.getByRole('heading', { name: 'Maria' })).toBeVisible()
    await expect(win.getByText('Pending DID Confirmation')).toBeVisible()

    expect(consoleErrors).toEqual([])
  })
})

test.describe('My Network app panel', () => {
  test('desktop: relationship collections import and merge contacts', async ({ page }) => {
    const consoleErrors: string[] = []
    page.on('console', (message) => {
      if (message.type() === 'error') consoleErrors.push(message.text())
    })

    await page.setViewportSize({ width: 1440, height: 900 })
    await page.addInitScript(placeMyNetworkOnFirstPage)
    await page.goto('/?scenario=normal')

    await page.getByTestId('desktop-app-my-network').click()
    const win = page.getByTestId('window-my-network')
    await expect(win).toBeVisible()

    await expect(win.getByRole('complementary').getByText('My Network')).toBeVisible()
    const networkSidebar = win.getByRole('complementary')
    await expect(networkSidebar.getByRole('button', { name: /Contacts/ })).toBeVisible()
    await expect(networkSidebar.getByRole('button', { name: /Friends/ })).toBeVisible()
    await expect(networkSidebar.getByRole('button', { name: /Joined Groups/ })).toBeVisible()

    await expect(win.getByPlaceholder('Search Contacts…')).toBeVisible()
    const graceTelegramRow = win.getByRole('button', { name: /Grace\.telegram/ }).first()
    await expect(graceTelegramRow).toBeVisible()

    await graceTelegramRow.click()
    await expect(win.getByText('One-way relation')).toBeVisible()
    await expect(win.getByText('Merge candidate')).toBeVisible()
    await expect(win.getByText('Social Accounts')).toBeVisible()
    await win.getByRole('button', { name: 'Comment Login Flow' }).click()
    await expect(win.getByText('HomeStation Comment Authorization')).toBeVisible()
    await expect(win.getByText('Session Token = None')).toBeVisible()

    await win.getByRole('button', { name: 'Import', exact: true }).click()
    await page.getByRole('button', { name: 'Start import' }).click()
    await expect(page.getByText('Import complete. Two contacts were added to Contacts.')).toBeVisible()
    await page.getByRole('button', { name: 'Done' }).click()
    await expect(win.getByRole('button', { name: /Mina\.imported/ })).toBeVisible()

    await graceTelegramRow.locator('input[type="checkbox"]').check()
    await win.getByRole('button', { name: /Grace\.imported/ }).locator('input[type="checkbox"]').check()
    await expect(win.getByText('2 selected')).toBeVisible()
    await win.getByRole('button', { name: 'Merge', exact: true }).click()
    const mergeDialog = page.getByRole('dialog', { name: 'Merge contacts' })
    await expect(mergeDialog).toBeVisible()
    await mergeDialog.getByRole('button', { name: 'Merge' }).click()
    await expect(win.getByRole('button', { name: /Grace\.imported/ })).toHaveCount(0)

    await win.getByRole('button', { name: 'Add Contact' }).click()
    await expect(win.getByRole('heading', { name: 'Nora' })).toBeVisible()

    await win.getByRole('button', { name: 'Add Collection' }).click()
    const collectionDialog = page.getByRole('dialog', { name: 'Add Collection' })
    await expect(collectionDialog).toBeVisible()
    await collectionDialog.getByRole('button', { name: 'Import Group' }).click()
    await collectionDialog.getByLabel('Group Name').fill('Imported Leads')
    await collectionDialog.getByRole('button', { name: 'Create' }).click()
    await expect(win.getByRole('button', { name: /Imported Leads/ })).toBeVisible()

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
    await expect(page.getByText('Self-hosted Groups', { exact: true })).toBeVisible()
    await expect(page.getByText('Collections', { exact: true })).toHaveCount(0)
    await page.getByLabel('Search or filter').click()
    await page.getByPlaceholder('Search name, DID, role, tag, status...').fill('Carol')
    await page.getByRole('button', { name: /Carol/ }).click()
    await expect(page.getByText('Pending DID Confirmation')).toBeVisible()

    const overflow = await page.evaluate(() => ({
      horizontal: document.documentElement.scrollWidth > window.innerWidth,
      vertical: document.documentElement.scrollHeight > window.innerHeight,
    }))
    expect(overflow.horizontal).toBeFalsy()
    expect(overflow.vertical).toBeFalsy()
    expect(consoleErrors).toEqual([])
  })
})

import { expect, test, type Page } from '@playwright/test'

// An 8×8 PNG, small enough for both the avatar and the Zone icon limits.
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEUlEQVR4nGN4VmGDFTEMLQkAN6BmgaudJikAAAAASUVORK5CYII=', 'base64')
const MOCK_CONTROL_PANEL_KEY = 'buckyos.mock.control-panel.v1'

const favicon = (page: Page) => page.locator('link[rel~="icon"]').getAttribute('href')

async function openZoneSettings(page: Page) {
  await page.getByTestId('desktop-app-settings').click()
  const win = page.getByTestId('window-settings')
  await win.getByRole('button', { name: 'Zone Name & Icon' }).click()
  await expect(win.getByTestId('settings-zone-page')).toBeVisible()
  return win
}

test.describe('object presentation', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 })
  })

  test('Zone name and icon: saved in settings, shown in the tab, favicon and sign-in page, and restorable', async ({ page, context }) => {
    await page.goto('/?scenario=normal')
    // Not configured: the readable part of the Zone DID (did:bns:alice), the static favicon.
    await expect(page).toHaveTitle('alice')
    expect(await favicon(page)).toBe('/favicon.svg')

    const win = await openZoneSettings(page)
    await expect(win.getByTestId('settings-zone-did')).toHaveText('did:bns:alice')
    await win.getByTestId('settings-zone-name').fill('我的家庭云')
    await win.getByTestId('settings-zone-description').fill('Photos and files for the family')
    await win.getByTestId('settings-zone-icon-input').setInputFiles({ name: 'zone.png', mimeType: 'image/png', buffer: PNG })
    await expect(win.getByTestId('settings-zone-preview')).toContainText('我的家庭云')
    await win.getByTestId('settings-zone-save').click()
    await expect(win.getByTestId('settings-zone-status')).toContainText('Saved')

    // The tab and favicon follow at once, without a reload.
    await expect(page).toHaveTitle('我的家庭云')
    await expect.poll(() => favicon(page)).toMatch(/^data:image\/png;base64,/)
    await expect(page.locator('link[rel~="icon"]')).toHaveAttribute('type', 'image/png')

    // A new tab (signed-out sign-in page included) reads the stored setting.
    const login = await context.newPage()
    await login.goto('/login')
    await expect(login.getByRole('heading', { name: 'Sign in to 我的家庭云' })).toBeVisible()
    await expect(login.getByTestId('login-zone-description')).toHaveText('Photos and files for the family')
    await expect(login).toHaveTitle('Sign in · 我的家庭云')
    await expect.poll(() => favicon(login)).toMatch(/^data:image\/png;base64,/)
    await login.close()

    // Standalone apps: "App · Zone".
    const hub = await context.newPage()
    await hub.goto('/messagehub')
    await expect(hub).toHaveTitle('MessageHub · 我的家庭云')
    await hub.close()

    // Restore defaults: back to the DID fallback and the static favicon.
    await win.getByTestId('settings-zone-reset').click()
    await expect(page).toHaveTitle('alice')
    await expect.poll(() => favicon(page)).toBe('/favicon.svg')
    await page.reload()
    await expect(page).toHaveTitle('alice')
  })

  test('a non-administrator sees the Zone display settings read-only', async ({ page }) => {
    await page.addInitScript((key) => {
      window.localStorage.setItem(key, JSON.stringify({ account: { user_id: 'bob', user_name: 'Bob', user_type: 'user', did: 'did:bns:bob' } }))
    }, MOCK_CONTROL_PANEL_KEY)
    await page.goto('/?scenario=normal')
    const win = await openZoneSettings(page)
    await expect(win.getByText('Only Zone administrators can change these settings.')).toBeVisible()
    await expect(win.getByTestId('settings-zone-name')).toBeDisabled()
    await expect(win.getByTestId('settings-zone-save')).toHaveCount(0)
  })

  test('the profile picture is uploaded, kept after a reload, shown in the taskbar, and removable', async ({ page }) => {
    await page.goto('/?scenario=normal')
    const taskbarAvatar = page.getByRole('button', { name: 'BuckyOS' }).locator('img')
    await expect(taskbarAvatar).toHaveCount(0)

    await page.getByRole('button', { name: 'BuckyOS' }).hover()
    await page.getByTestId('taskbar-profile-card').getByRole('button', { name: 'Manage account' }).click()
    const win = page.getByTestId('window-users-agents')
    await win.getByRole('button', { name: 'Avatar' }).click()
    const dialog = page.getByRole('dialog', { name: 'Profile picture' })
    await dialog.getByTestId('self-avatar-input').setInputFiles({ name: 'me.png', mimeType: 'image/png', buffer: PNG })
    await dialog.getByTestId('self-avatar-save').click()
    await expect(dialog).toHaveCount(0)

    const header = win.locator('img[src^="data:image/jpeg"]').first()
    await expect(header).toBeVisible()
    await expect(taskbarAvatar).toHaveAttribute('src', /^data:image\/jpeg;base64,/)

    await page.reload()
    await expect(page.getByRole('button', { name: 'BuckyOS' }).locator('img')).toHaveAttribute('src', /^data:image\/jpeg;base64,/)

    await page.getByRole('button', { name: 'BuckyOS' }).hover()
    await page.getByTestId('taskbar-profile-card').getByRole('button', { name: 'Manage account' }).click()
    await win.getByRole('button', { name: 'Avatar' }).click()
    await dialog.getByRole('button', { name: 'Remove' }).click()
    await dialog.getByTestId('self-avatar-save').click()
    await expect(dialog).toHaveCount(0)
    await expect(page.getByRole('button', { name: 'BuckyOS' }).locator('img')).toHaveCount(0)
  })

  test('a broken picture falls back to the default without a broken image', async ({ page }) => {
    await page.addInitScript((key) => {
      window.localStorage.setItem(key, JSON.stringify({
        account: { user_id: 'alice', user_name: 'Alice', user_type: 'admin', did: 'did:bns:alice', profile: { avatar: 'data:image/png;base64,broken' } },
      }))
    }, MOCK_CONTROL_PANEL_KEY)
    await page.goto('/?scenario=normal')
    const avatar = page.getByRole('button', { name: 'BuckyOS' })
    await expect(avatar).toBeVisible()
    await expect(avatar.locator('img')).toHaveCount(0)
    await expect(avatar).toContainText('A')
  })
})

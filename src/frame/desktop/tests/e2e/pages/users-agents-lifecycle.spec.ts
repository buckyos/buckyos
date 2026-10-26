import { expect, test, type Locator, type Page } from '@playwright/test'

const MOCK_CONTROL_PANEL = 'buckyos.mock.control-panel.v1'
const browserErrors = new WeakMap<Page, string[]>()

declare global {
  interface Window {
    __userLifecycle: {
      calls: Array<{ method: string; params: Record<string, unknown> }>
      fail?: string
      holdDelete?: boolean
      releaseDelete?: () => void
      warn?: string
      loseResponse?: string
      failAfterCommit?: string
    }
  }
}

async function installRpcControls(page: Page) {
  await page.route('**/kapi/verify-hub', async route => {
    const request = route.request().postDataJSON()
    expect(request.method).toBe('sudo_by_password')
    expect(request.params.aud).toBe('system-config')
    await route.fulfill({ json: {
      result: { session_token: `lifecycle.${Buffer.from(JSON.stringify({ exp: Math.floor(Date.now() / 1000) + 180 })).toString('base64url')}.signature` },
      sys: [request.sys[0]],
    } })
  })
  await page.route('**/src/api/control_panel_mock.ts*', async route => {
    if (new URL(route.request().url()).searchParams.has('lifecycle-original')) {
      await route.continue()
      return
    }
    await route.fulfill({
      contentType: 'application/javascript',
      body: `
        import { callMockControlPanel as original } from '/src/api/control_panel_mock.ts?lifecycle-original';
        export * from '/src/api/control_panel_mock.ts?lifecycle-original';
        window.__userLifecycle = { calls: [] };
        export async function callMockControlPanel(method, params) {
          const control = window.__userLifecycle;
          control.calls.push({ method, params });
          if (control.fail === method) return { data: null, error: new Error(method + ' unavailable for lifecycle test') };
          if (method === 'user.delete' && control.holdDelete) {
            await new Promise(resolve => { control.releaseDelete = resolve; });
          }
          const result = await original(method, params);
          if (result.data && control.loseResponse === method) {
            control.fail = control.failAfterCommit;
            return { data: null, error: new Error(method + ' response lost after commit') };
          }
          if (result.data && control.warn === method) {
            return { data: { ...result.data, rbac_refreshed: false, warning: 'Account state saved; permission refresh is pending.' }, error: null };
          }
          return result;
        }
      `,
    })
  })
  page.on('dialog', dialog => void dialog.accept())
}

async function openUsersAgents(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-users-agents').click()
  const win = page.getByTestId('window-users-agents')
  await expect(win.locator('[data-entity-id="bob"]')).toBeVisible()
  return win
}

async function selectBob(win: Locator) {
  await win.locator('[data-entity-id="bob"]').click()
  await expect(win.getByTestId('local-user-delete')).toBeVisible()
}

async function approveSudo(page: Page, title: string, confirm: string) {
  const dialog = page.getByRole('dialog', { name: title, exact: true })
  await expect(dialog).toBeVisible()
  await dialog.getByLabel('Password', { exact: true }).fill('test-admin-password')
  await dialog.getByRole('button', { name: confirm, exact: true }).click()
  await expect(dialog).toHaveCount(0)
}

async function storedBob(page: Page) {
  return page.evaluate(key => JSON.parse(localStorage.getItem(key)!).zone_users.bob, MOCK_CONTROL_PANEL)
}

async function openDeleted(win: Locator) {
  await win.getByTestId('users-agents-deleted-users').click()
  const deleted = win.getByTestId('deleted-users-page')
  await expect(deleted).toBeVisible()
  return deleted
}

test.beforeEach(async ({ page }) => {
  const errors: string[] = []
  browserErrors.set(page, errors)
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()) })
  page.on('pageerror', error => errors.push(String(error)))
  await installRpcControls(page)
})

test.afterEach(async ({ page }) => {
  expect(browserErrors.get(page)).toEqual([])
})

test('deletion waits for the RPC, survives reload, and restores the original account', async ({ page }) => {
  let win = await openUsersAgents(page)
  const original = await storedBob(page)
  await selectBob(win)
  await page.evaluate(() => { window.__userLifecycle.holdDelete = true })
  await win.getByTestId('local-user-delete').click()
  await approveSudo(page, 'Remove user', 'Remove User')
  await expect.poll(() => page.evaluate(() => Boolean(window.__userLifecycle.releaseDelete))).toBe(true)
  await expect(win.getByTestId('local-user-delete')).toBeDisabled()
  await expect(win.locator('[data-entity-id="bob"]')).toBeVisible()
  expect(await page.evaluate(() => window.__userLifecycle.calls.filter(call => call.method === 'user.delete').length)).toBe(1)
  expect((await storedBob(page)).state ?? 'active').toBe('active')

  await page.evaluate(() => window.__userLifecycle.releaseDelete?.())
  await expect(win.locator('[data-entity-id="bob"]')).toHaveCount(0)
  expect(await storedBob(page)).toEqual({ ...original, state: 'deleted' })
  await page.reload()
  await page.getByTestId('desktop-app-users-agents').click()
  win = page.getByTestId('window-users-agents')
  await expect(win.getByTestId('users-agents-deleted-users')).toBeVisible()
  await expect(win.locator('[data-entity-id="bob"]')).toHaveCount(0)

  const deleted = await openDeleted(win)
  const row = deleted.locator('[data-user-id="bob"]')
  await expect(row).toBeVisible()
  await row.getByRole('button', { name: 'Restore original account' }).click()
  await approveSudo(page, 'Restore original account', 'Restore original account')
  await expect(row).toHaveCount(0)
  await expect(win.locator('[data-entity-id="bob"]')).toBeVisible()
  expect(await storedBob(page)).toEqual({ ...original, state: 'active' })
})

test('a rejected deletion retains the user and displays the server error', async ({ page }) => {
  const win = await openUsersAgents(page)
  await selectBob(win)
  await page.evaluate(() => { window.__userLifecycle.fail = 'user.delete' })
  await win.getByTestId('local-user-delete').click()
  await approveSudo(page, 'Remove user', 'Remove User')
  await expect(win.getByRole('alert')).toContainText('user.delete unavailable for lifecycle test')
  await expect(win.locator('[data-entity-id="bob"]')).toBeVisible()
  await expect(win.getByTestId('local-user-delete')).toBeEnabled()
  expect((await storedBob(page)).state ?? 'active').toBe('active')
})

test('a failed list read after deletion preserves the snapshot and retries only the read', async ({ page }) => {
  const win = await openUsersAgents(page)
  await selectBob(win)
  await page.evaluate(() => { window.__userLifecycle.fail = 'user.list' })
  await win.getByTestId('local-user-delete').click()
  await approveSudo(page, 'Remove user', 'Remove User')
  await expect(win.getByRole('alert')).toContainText('user.list unavailable for lifecycle test')
  await expect(win.getByRole('alert')).toContainText(/deleted/i)
  await expect(win.locator('[data-entity-id="bob"]')).toBeVisible()
  await expect(win.locator('[data-entity-id="carol"]')).toBeVisible()
  expect((await storedBob(page)).state).toBe('deleted')
  await page.evaluate(() => { delete window.__userLifecycle.fail })
  await win.getByRole('button', { name: 'Refresh list', exact: true }).click()
  await expect(win.locator('[data-entity-id="bob"]')).toHaveCount(0)
  expect(await page.evaluate(() => window.__userLifecycle.calls.filter(call => call.method === 'user.delete').length)).toBe(1)
})

test('permission refresh warnings remain visible after a successful deletion', async ({ page }) => {
  const win = await openUsersAgents(page)
  await selectBob(win)
  await page.evaluate(() => { window.__userLifecycle.warn = 'user.delete' })
  await win.getByTestId('local-user-delete').click()
  await approveSudo(page, 'Remove user', 'Remove User')
  await expect(win.locator('[data-entity-id="bob"]')).toHaveCount(0)
  await expect(win.getByTestId('user-lifecycle-notice')).toContainText('Account state saved; permission refresh is pending.')
  expect((await storedBob(page)).state).toBe('deleted')
})

test('a lost delete response is reconciled with the persisted deleted state', async ({ page }) => {
  const win = await openUsersAgents(page)
  await selectBob(win)
  await page.evaluate(() => { window.__userLifecycle.loseResponse = 'user.delete' })
  await win.getByTestId('local-user-delete').click()
  await approveSudo(page, 'Remove user', 'Remove User')
  await expect(win.locator('[data-entity-id="bob"]')).toHaveCount(0)
  await expect(win.getByTestId('user-lifecycle-notice')).toBeVisible()
  expect((await storedBob(page)).state).toBe('deleted')
  expect(await page.evaluate(() => window.__userLifecycle.calls.some(call => call.method === 'user.list' && call.params.include_deleted === true))).toBe(true)
  expect(await page.evaluate(() => window.__userLifecycle.calls.filter(call => call.method === 'user.delete').length)).toBe(1)
})

test('an uncertain delete blocks another mutation until a read-only refresh resolves it', async ({ page }) => {
  const win = await openUsersAgents(page)
  await selectBob(win)
  await page.evaluate(() => {
    window.__userLifecycle.loseResponse = 'user.delete'
    window.__userLifecycle.failAfterCommit = 'user.list'
  })
  await win.getByTestId('local-user-delete').click()
  await approveSudo(page, 'Remove user', 'Remove User')
  await expect(win.getByRole('button', { name: 'Refresh list', exact: true })).toBeVisible()
  await expect(win.getByTestId('local-user-delete')).toBeDisabled()
  await expect(win.locator('[data-entity-id="bob"]')).toBeVisible()
  expect((await storedBob(page)).state).toBe('deleted')
  await page.evaluate(() => { delete window.__userLifecycle.fail })
  await win.getByRole('button', { name: 'Refresh list', exact: true }).click()
  await expect(win.locator('[data-entity-id="bob"]')).toHaveCount(0)
  expect(await page.evaluate(() => window.__userLifecycle.calls.filter(call => call.method === 'user.delete').length)).toBe(1)
})

test('a deleted username directs creation to explicit recovery without replacing the account', async ({ page }) => {
  const win = await openUsersAgents(page)
  await selectBob(win)
  await win.getByTestId('local-user-delete').click()
  await approveSudo(page, 'Remove user', 'Remove User')
  await expect(win.locator('[data-entity-id="bob"]')).toHaveCount(0)
  const deletedAccount = await storedBob(page)
  await win.getByLabel('Add User').click()
  await win.getByLabel('Local username').fill('bob')
  await win.getByLabel('Display name').fill('A different Bob')
  await win.getByLabel('Initial password').fill('replacement-password')
  await win.getByLabel('Confirm password').fill('replacement-password')
  await win.getByRole('button', { name: 'Next' }).click()
  await win.getByRole('button', { name: 'Create User', exact: true }).click()
  await approveSudo(page, 'Create local user', 'Create user')
  await expect(win.getByRole('button', { name: 'Open deleted users' })).toBeVisible()
  expect(await storedBob(page)).toEqual(deletedAccount)
  await win.getByRole('button', { name: 'Open deleted users' }).click()
  await expect(win.getByTestId('deleted-users-page').locator('[data-user-id="bob"]')).toBeVisible()
})

test('a failed restore retains the deleted account and displays its error', async ({ page }) => {
  const win = await openUsersAgents(page)
  await selectBob(win)
  await win.getByTestId('local-user-delete').click()
  await approveSudo(page, 'Remove user', 'Remove User')
  await expect(win.locator('[data-entity-id="bob"]')).toHaveCount(0)
  const deleted = await openDeleted(win)
  await page.evaluate(() => { window.__userLifecycle.fail = 'user.change_state' })
  const row = deleted.locator('[data-user-id="bob"]')
  await row.getByRole('button', { name: 'Restore original account' }).click()
  await approveSudo(page, 'Restore original account', 'Restore original account')
  await expect(deleted.getByRole('alert').filter({ hasText: 'user.change_state unavailable for lifecycle test' })).toBeVisible()
  await expect(row).toBeVisible()
  expect((await storedBob(page)).state).toBe('deleted')
})

test('the current account and root have no removal action', async ({ page }) => {
  await page.addInitScript(key => localStorage.setItem(key, JSON.stringify({ zone_users: {
    alice: { user_type: 'admin' },
    bob: { user_type: 'user' },
    root: { user_type: 'root' },
  } })), MOCK_CONTROL_PANEL)
  const win = await openUsersAgents(page)
  await win.locator('[data-entity-id="alice"]').click()
  await expect(win.getByRole('heading', { name: 'Profile' })).toBeVisible()
  await expect(win.getByTestId('local-user-delete')).toHaveCount(0)
  await expect(win.locator('[data-entity-id="root"]')).toHaveCount(0)
})

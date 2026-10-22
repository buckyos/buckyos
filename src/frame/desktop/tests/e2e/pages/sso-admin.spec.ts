import { expect, test, type Locator, type Page } from '@playwright/test'

const MOCK_CONTROL_PANEL = 'buckyos.mock.control-panel.v1'

async function seedControlPanel(page: Page, state: Record<string, unknown>) {
  await page.addInitScript(([key, value]) => {
    if (!window.sessionStorage.getItem('sso-seeded')) {
      window.localStorage.setItem(key, value)
      window.sessionStorage.setItem('sso-seeded', '1')
    }
  }, [MOCK_CONTROL_PANEL, JSON.stringify(state)] as const)
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const mockState = (page: Page): Promise<any> =>
  page.evaluate((key) => JSON.parse(localStorage.getItem(key) ?? '{}'), MOCK_CONTROL_PANEL)

function trackConsole(page: Page) {
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(String(error)))
  return errors
}

async function openSettings(page: Page) {
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-settings').click()
  const win = page.getByTestId('window-settings')
  await expect(win.getByTestId('settings-general-page')).toBeVisible()
  return win
}

async function openInvites(page: Page): Promise<Locator> {
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-users-agents').click()
  const win = page.getByTestId('window-users-agents')
  await win.getByTestId('users-agents-exclusive-invites').click()
  const invites = win.getByTestId('exclusive-invites-page')
  await expect(invites).toBeVisible()
  return invites
}

test.describe('settings: sign-in and sharing', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 860 })
  })

  test('admin switches DID login off and sets the jump entry; the entry page follows', async ({ page }) => {
    const errors = trackConsole(page)
    const win = await openSettings(page)
    await win.getByRole('button', { name: 'Sign-in & Sharing', exact: true }).click()
    const ssoPage = win.getByTestId('settings-sso-page')
    await expect(ssoPage.getByRole('switch', { name: 'Self-registration' })).toBeChecked()
    const didLogin = ssoPage.getByRole('switch', { name: 'DID login' })
    await expect(didLogin).toBeChecked()
    await expect(ssoPage.getByTestId('settings-sso-jump')).toHaveAttribute('placeholder', 'https://jump.buckyos.org')

    await didLogin.uncheck()
    await ssoPage.getByTestId('settings-sso-jump').fill('http://jump.example.com')
    await expect(ssoPage.getByTestId('settings-sso-save')).toBeDisabled()
    await ssoPage.getByTestId('settings-sso-jump').fill('https://jump.example.com')
    await ssoPage.getByTestId('settings-sso-save').click()
    await expect(ssoPage.getByTestId('settings-sso-status')).toHaveText('Saved.')
    expect((await mockState(page)).sso.settings).toEqual({ self_register_enabled: true, did_login_enabled: false, jump_base_url: 'https://jump.example.com' })

    await page.evaluate((key) => {
      const state = JSON.parse(localStorage.getItem(key) ?? '{}')
      localStorage.setItem(key, JSON.stringify({ ...state, session: { kind: 'anonymous' } }))
    }, MOCK_CONTROL_PANEL)
    await page.goto('/sso/entry?service=aiworkspace&entry=se-dj')
    await expect(page.getByTestId('sso-entry-method-did_login')).toBeDisabled()
    expect(errors).toEqual([])
  })

  test('is hidden from non-admins', async ({ page }) => {
    await seedControlPanel(page, { account: { user_id: 'bob', user_name: 'Bob', user_type: 'user', did: 'did:bns:bob' } })
    const win = await openSettings(page)
    await expect(win.getByRole('button', { name: 'Privacy', exact: true })).toBeVisible()
    await expect(win.getByRole('button', { name: 'Sign-in & Sharing', exact: true })).toHaveCount(0)
  })
})

test.describe('users and agents: exclusive invites', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    page.on('dialog', (dialog) => void dialog.accept())
  })

  test('admin creates an invite, sees the link once, retries the session invitation, rotates and revokes', async ({ page }) => {
    const errors = trackConsole(page)
    const invites = await openInvites(page)
    const list = invites.getByTestId('exclusive-invite-list')
    await expect(list.getByTestId('exclusive-invite-row')).toHaveCount(3)

    const form = invites.getByTestId('exclusive-invite-create')
    await form.getByLabel('Account ID').fill('cust-9')
    await form.getByLabel('Display name').fill('Customer Nine')
    await form.getByTestId('exclusive-invite-address').fill('carol@acme.com')
    await expect(form).toContainText('Only "c***@acme.com" is stored.')
    await form.getByLabel('Resource (optional)').click()
    await page.getByRole('option', { name: 'Message Hub session' }).click()
    await form.getByLabel('Group DID').fill('did:bns:support.offline')
    await form.getByLabel('Session ID').fill('t-9')
    await form.getByLabel('Title the customer sees').fill('Ticket #9')
    await form.getByTestId('exclusive-invite-submit').click()

    const once = invites.getByTestId('exclusive-invite-link')
    await expect(once).toContainText('shown only now')
    const firstLink = (await once.getByTestId('exclusive-invite-link-value').textContent()) ?? ''
    expect(firstLink).toMatch(/\/sso\/invite\/ei-[0-9a-f]+#s=[0-9a-f]+$/)
    await once.getByTestId('exclusive-invite-link-done').click()
    await expect(once).toHaveCount(0)

    const state = await mockState(page)
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const created = state.sso.invites.find((invite: any) => invite.user_id === 'cust-9')
    expect(created.channel).toEqual({ kind: 'email', target_hint: 'c***@acme.com' })
    expect(JSON.stringify(state)).not.toContain('carol@acme.com')
    expect(state.sso.accounts['cust-9']).toEqual({ show_name: 'Customer Nine', password_set: false, state: 'active' })

    const row = list.locator(`[data-invite-id="${created.invite_id}"]`)
    await expect(row.getByTestId('exclusive-invite-resource-pending')).toBeVisible()
    await row.getByTestId('exclusive-invite-retry').click()
    await expect(row.getByTestId('exclusive-invite-resource-pending')).toHaveCount(0)

    await row.getByTestId('exclusive-invite-rotate').click()
    const rotated = invites.getByTestId('exclusive-invite-link')
    await expect(rotated).toBeVisible()
    const secondLink = (await rotated.getByTestId('exclusive-invite-link-value').textContent()) ?? ''
    expect(secondLink).not.toBe(firstLink)
    expect(secondLink.split('#')[0]).toBe(firstLink.split('#')[0])

    await row.getByTestId('exclusive-invite-revoke').click()
    await expect(row.getByTestId('exclusive-invite-status')).toHaveText('Revoked')
    await expect(row.getByTestId('exclusive-invite-rotate')).toBeDisabled()
    await expect(row.getByTestId('exclusive-invite-revoke')).toBeDisabled()
    expect(errors).toEqual([])
  })

  test('a customer resend request shows as a badge; a password account cannot get a new link', async ({ page }) => {
    // The customer asks for a new link of the expired invite first.
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto('/sso/invite/ei-expired#s=mock-secret-expired')
    await page.getByTestId('sso-invite-resend').click()
    await expect(page.getByTestId('sso-invite-resend-result')).toHaveAttribute('data-status', 'submitted')
    await page.evaluate((key) => {
      const state = JSON.parse(localStorage.getItem(key) ?? '{}')
      delete state.session
      localStorage.setItem(key, JSON.stringify(state))
    }, MOCK_CONTROL_PANEL)

    const invites = await openInvites(page)
    const expired = invites.locator('[data-invite-id="ei-expired"]')
    await expect(expired.getByTestId('exclusive-invite-status')).toHaveText('Link expired')
    await expect(expired.getByTestId('exclusive-invite-resend-badge')).toContainText('Customer asked for a new link')
    await expired.getByTestId('exclusive-invite-rotate').click()
    await expect(invites.getByTestId('exclusive-invite-link')).toBeVisible()
    await expect(expired.getByTestId('exclusive-invite-resend-badge')).toHaveCount(0)

    const withPassword = invites.locator('[data-invite-id="ei-password"]')
    await expect(withPassword.getByTestId('exclusive-invite-status')).toHaveText('Password set; the link is only an entry')
    await expect(withPassword.getByTestId('exclusive-invite-rotate')).toBeDisabled()
  })

  test('reusing an existing restricted account', async ({ page }) => {
    const invites = await openInvites(page)
    const form = invites.getByTestId('exclusive-invite-create')
    await form.getByTestId('exclusive-invite-mode-existing').click()
    await form.getByLabel('Restricted account').click()
    await page.getByRole('option', { name: 'acme-c1' }).click()
    await form.getByLabel('Channel').click()
    await page.getByRole('option', { name: 'SMS' }).click()
    await form.getByTestId('exclusive-invite-address').fill('13812345678')
    await form.getByTestId('exclusive-invite-submit').click()
    await expect(invites.getByTestId('exclusive-invite-link')).toBeVisible()
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const reused = (await mockState(page)).sso.invites.filter((invite: any) => invite.user_id === 'acme-c1')
    expect(reused).toHaveLength(2)
    expect(reused[1].channel).toEqual({ kind: 'sms', target_hint: '138****5678' })
  })
})

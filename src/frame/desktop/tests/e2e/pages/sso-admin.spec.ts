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

  async function openProviders(page: Page) {
    const win = await openSettings(page)
    await win.getByRole('button', { name: 'Sign-in & Sharing', exact: true }).click()
    const section = win.getByTestId('settings-sso-providers')
    await expect(section).toBeVisible()
    return section
  }

  const googleSource = { kind: 'google', id: 'google', display_name: 'Google', enabled: true, client_id: 'old.apps.googleusercontent.com', secret_ref: 'existing-google' }
  const acmeSource = {
    kind: 'oidc', id: 'acme', display_name: 'Acme SSO', enabled: true, client_id: 'desktop', secret_ref: 'acme',
    webhost: 'acme.example', issuer: 'https://login.acme.example', authorization_endpoint: 'https://login.acme.example/authorize',
    token_endpoint: 'https://login.acme.example/token', jwks_uri: 'https://login.acme.example/keys', scopes: ['openid', 'email'],
  }

  test('adds Google in two steps: what to prepare, then the form', async ({ page }) => {
    const errors = trackConsole(page)
    const section = await openProviders(page)
    await expect(section).toContainText('No external login provider yet.')
    await section.getByTestId('sso-provider-add').click()

    const dialog = page.getByTestId('sso-provider-dialog')
    await expect(dialog).toContainText('Step 1 of 2')
    await expect(dialog.getByRole('radio', { name: 'Google' })).toBeChecked()
    await expect(dialog).toContainText('/sso/oauth/google/callback')
    await expect(dialog).toContainText('$BUCKYOS_ROOT/security/oauth/google-demo')
    await expect(dialog).toContainText('web.client_secret')
    await dialog.getByRole('radio', { name: 'OpenID Connect' }).check()
    await expect(dialog).toContainText('.well-known/openid-configuration')
    await dialog.getByRole('radio', { name: 'Google' }).check()
    await dialog.getByTestId('sso-provider-ready').click()

    await expect(dialog).toContainText('Step 2 of 2')
    await expect(dialog.getByRole('textbox', { name: 'Secret file name' })).toHaveCount(0)
    await dialog.getByTestId('sso-provider-save').click()
    await expect(dialog).toContainText('Required')
    await dialog.getByRole('textbox', { name: 'Client ID', exact: true }).fill('test.apps.googleusercontent.com')
    await dialog.getByTestId('sso-provider-save').click()
    await expect(dialog).toBeHidden()

    await expect(section.getByTestId('sso-provider-row-google')).toContainText('Google')
    expect((await mockState(page)).sso.settings.oauth_sources).toMatchObject([
      { kind: 'google', id: 'google', client_id: 'test.apps.googleusercontent.com', secret_ref: 'google-demo', enabled: true },
    ])

    // A Zone has one Google source.
    await section.getByTestId('sso-provider-add').click()
    await expect(dialog.getByRole('radio', { name: 'Google' })).toBeDisabled()
    await expect(dialog.getByRole('radio', { name: 'OpenID Connect' })).toBeChecked()
    expect(errors).toEqual([])
  })

  test('adds an OpenID Connect source after checking its fields', async ({ page }) => {
    const section = await openProviders(page)
    await section.getByTestId('sso-provider-add').click()
    const dialog = page.getByTestId('sso-provider-dialog')
    await dialog.getByRole('radio', { name: 'OpenID Connect' }).check()
    await dialog.getByTestId('sso-provider-ready').click()

    const field = (name: string) => dialog.getByTestId(`sso-provider-field-${name}`)
    await field('id').fill('Acme')
    await expect(dialog).toContainText('Lowercase letters, digits, - and _ only')
    await field('id').fill('acme')
    await expect(dialog.getByTestId('sso-provider-callback')).toContainText('/sso/oauth/acme/callback')
    await field('display_name').fill('Acme SSO')
    await field('client_id').fill('desktop')
    await field('secret_ref').fill('acme')
    await expect(dialog).toContainText('$BUCKYOS_ROOT/security/oauth/acme')
    await field('webhost').fill('acme.example')
    await field('issuer').fill('http://login.acme.example')
    await expect(dialog).toContainText('Use an https:// address')
    await field('issuer').fill('https://login.acme.example')
    await field('authorization_endpoint').fill('https://login.acme.example/authorize')
    await field('token_endpoint').fill('https://login.acme.example/token')
    await dialog.getByTestId('sso-provider-save').click()
    await expect(dialog).toContainText('Required')
    await field('jwks_uri').fill('https://login.acme.example/keys')
    await dialog.getByTestId('sso-provider-save').click()
    await expect(dialog).toBeHidden()

    await expect(section.getByTestId('sso-provider-row-acme')).toContainText('OpenID Connect · acme.example')
    expect((await mockState(page)).sso.settings.oauth_sources).toMatchObject([{ ...acmeSource, scopes: [] }])
  })

  test('edits a source; the fields that name its accounts are locked', async ({ page }) => {
    await seedControlPanel(page, { sso: { settings: { self_register_enabled: true, did_login_enabled: true, oauth_sources: [googleSource, acmeSource] } } })
    const section = await openProviders(page)
    const dialog = page.getByTestId('sso-provider-dialog')

    await section.getByRole('button', { name: 'Edit Acme SSO' }).click()
    await expect(dialog).toContainText('Edit “Acme SSO”')
    for (const name of ['id', 'webhost', 'issuer']) {
      await expect(dialog.getByTestId(`sso-provider-field-${name}`)).toBeDisabled()
    }
    await dialog.getByTestId('sso-provider-field-display_name').fill('Acme')
    await dialog.getByTestId('sso-provider-save').click()
    await expect(dialog).toBeHidden()
    await expect(section.getByTestId('sso-provider-row-acme')).toContainText('Acme')

    // Google keeps its stored credential file name.
    await section.getByRole('button', { name: 'Edit Google' }).click()
    await expect(dialog).toContainText('$BUCKYOS_ROOT/security/oauth/existing-google')
    await dialog.getByRole('textbox', { name: 'Client ID', exact: true }).fill('new.apps.googleusercontent.com')
    await dialog.getByTestId('sso-provider-save').click()
    await expect(dialog).toBeHidden()
    expect((await mockState(page)).sso.settings.oauth_sources).toMatchObject([
      { id: 'google', client_id: 'new.apps.googleusercontent.com', secret_ref: 'existing-google' },
      { id: 'acme', display_name: 'Acme', webhost: 'acme.example' },
    ])

    await section.getByRole('switch', { name: 'Allow sign-in with Acme' }).uncheck()
    await expect(section.getByTestId('sso-provider-row-acme')).toContainText('Off')
    expect((await mockState(page)).sso.settings.oauth_sources[1].enabled).toBe(false)
  })

  test('deleting a source names the local accounts that sign in with it', async ({ page }) => {
    await seedControlPanel(page, { sso: {
      settings: { self_register_enabled: true, did_login_enabled: true, oauth_sources: [googleSource, acmeSource] },
      oauth_accounts: { google: [{ user_id: 'alice', show_name: 'Alice', email: 'alice@gmail.com', state: 'active' }] },
    } })
    const section = await openProviders(page)
    const dialog = page.getByTestId('sso-provider-delete-dialog')

    await section.getByRole('button', { name: 'Delete Google' }).click()
    await expect(dialog).toContainText('the users below can no longer sign in with Google')
    await expect(dialog.getByTestId('sso-provider-delete-accounts')).toContainText('Alice · alice')
    await expect(dialog.getByTestId('sso-provider-delete-accounts')).toContainText('alice@gmail.com')
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    await expect(dialog).toBeHidden()
    await expect(section.getByTestId('sso-provider-row-google')).toBeVisible()

    await section.getByRole('button', { name: 'Delete Acme SSO' }).click()
    await expect(dialog).toContainText('No local account signs in with Acme SSO')
    await dialog.getByTestId('sso-provider-delete-confirm').click()
    await expect(dialog).toBeHidden()
    await expect(section.getByTestId('sso-provider-row-acme')).toHaveCount(0)
    expect((await mockState(page)).sso.settings.oauth_sources).toMatchObject([{ id: 'google' }])
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

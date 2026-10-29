import { expect, test, type Page } from '@playwright/test'

/** The mock control panel (src/api/control_panel_mock.ts) merges this object over its defaults. */
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

const DJ_ENTRY = '/sso/entry?service=aiworkspace&entry=se-dj'

test.describe('share entry page', () => {
  test('offers only the entry methods and marks the ones the Zone closed', async ({ page }) => {
    await seedControlPanel(page, {
      session: { kind: 'anonymous' },
      sso: { settings: { self_register_enabled: true, did_login_enabled: false } },
    })
    await page.goto(DJ_ENTRY)
    await expect(page.getByRole('heading', { name: 'DJ Radio' })).toBeVisible()
    await expect(page.getByTestId('sso-entry-resource')).toContainText('AI Workspace · shared by DJ')

    const methods = page.getByTestId('sso-entry-methods')
    await expect(methods.getByTestId('sso-entry-method-guest')).toBeEnabled()
    await expect(methods.getByTestId('sso-entry-method-password')).toBeEnabled()
    await expect(methods.getByTestId('sso-entry-method-self_register')).toBeEnabled()
    await expect(methods.getByTestId('sso-entry-method-did_login')).toBeDisabled()
    await expect(methods.getByTestId('sso-entry-method-did_login-unavailable')).toHaveText('Not available right now')

    await methods.getByTestId('sso-entry-method-password').click()
    await page.waitForURL('**/login?**')
    const redirect = new URL(page.url()).searchParams.get('redirect_url') ?? ''
    expect(new URL(redirect).pathname).toBe('/sso/entry')
    expect(new URL(redirect).searchParams.get('join')).toBe('1')
    expect(new URL(redirect).searchParams.get('entry')).toBe('se-dj')
  })

  test('a Message Hub entry without Guest shows no guest button', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto(`/sso/entry?service=msg-center&entry=${encodeURIComponent('did:bns:support.alice')}`)
    await expect(page.getByRole('heading', { name: 'Acme Support' })).toBeVisible()
    await expect(page.getByTestId('sso-entry-method-password')).toBeVisible()
    await expect(page.getByTestId('sso-entry-method-guest')).toHaveCount(0)
  })

  test('an unavailable or unknown entry gives one neutral message', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    for (const path of ['/sso/entry?service=aiworkspace&entry=se-closed', '/sso/entry?service=aiworkspace&entry=nope', '/sso/entry?service=other&entry=se-dj']) {
      await page.goto(path)
      await expect(page.getByTestId('sso-entry-unavailable')).toHaveText('This link is not available. Contact the person who shared it.')
      await expect(page.getByTestId('sso-entry-methods')).toHaveCount(0)
    }
  })

  test('self-registration creates the account, signs in and enters the resource', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto(DJ_ENTRY)
    await page.getByTestId('sso-entry-method-self_register').click()
    const form = page.getByTestId('sso-register-form')
    await form.getByLabel('User name').fill('bob')
    await form.getByLabel('Password', { exact: true }).fill('correct-horse')
    await form.getByLabel('Confirm password').fill('correct-horse')
    await form.getByTestId('sso-register-submit').click()
    await expect(form.getByText('This user name is taken. Choose another one.')).toBeVisible()

    await form.getByLabel('User name').fill('dj-fan')
    await form.getByLabel('Display name (optional)').fill('DJ Fan')
    await form.getByTestId('sso-register-submit').click()
    await page.waitForURL('**/workspace')
    const state = await mockState(page)
    expect(state.sso.accounts['dj-fan']).toEqual({ show_name: 'DJ Fan', password_set: true, state: 'active' })
    expect(state.account.user_id).toBe('dj-fan')
    expect(state.session.kind).toBe('account')
  })

  test('a signed-in visitor continues as themselves or switches identity explicitly', async ({ page }) => {
    await page.goto(DJ_ENTRY)
    const signedIn = page.getByTestId('sso-entry-signed-in')
    await expect(signedIn.getByTestId('sso-identity-name')).toHaveText('Alice')
    await expect(signedIn.getByTestId('sso-entry-continue')).toHaveText('Continue as Alice')

    await signedIn.getByTestId('sso-entry-switch').click()
    await page.getByTestId('sso-entry-switch-cancel').click()
    await expect(signedIn).toBeVisible()
    expect((await mockState(page)).session).toBeUndefined()

    await signedIn.getByTestId('sso-entry-switch').click()
    await page.getByTestId('sso-entry-switch-confirm-button').click()
    await expect(page.getByTestId('sso-entry-methods')).toBeVisible()
    expect((await mockState(page)).session).toEqual({ kind: 'anonymous' })
  })

  test('coming back from a sign-in started here enters the resource', async ({ page }) => {
    await page.goto(DJ_ENTRY)
    const entry = new URL(page.url())
    await page.evaluate((key) => window.sessionStorage.setItem('buckyos.sso.pending-join', key), `${entry.searchParams.get('service')}|${entry.searchParams.get('entry')}`)
    await page.goto(`${DJ_ENTRY}&join=1`)
    await page.waitForURL('**/workspace')
  })

  test('a join=1 link that no sign-in here produced still asks to continue', async ({ page }) => {
    await page.goto(`${DJ_ENTRY}&join=1`)
    const signedIn = page.getByTestId('sso-entry-signed-in')
    await expect(signedIn.getByTestId('sso-entry-continue')).toBeEnabled()
    expect(new URL(page.url()).pathname).toBe('/sso/entry')
  })

  test('DID login goes to the DID home Zone; an unreachable home Zone is reported', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.route('https://bob.web3.buckyos.ai/**', (route) => route.fulfill({ status: 200, contentType: 'text/html', body: '<p>canonical</p>' }))
    await page.goto(DJ_ENTRY)
    await page.getByTestId('sso-entry-method-did_login').click()
    await page.getByTestId('sso-did-input').fill('did:bns:offline-bob')
    await page.getByTestId('sso-did-submit').click()
    await expect(page.getByTestId('sso-did-unavailable')).toBeVisible()

    await page.getByTestId('sso-did-input').fill('did:bns:bob')
    await page.getByTestId('sso-did-submit').click()
    await page.waitForURL('https://bob.web3.buckyos.ai/sso/federate?**')
    const url = new URL(page.url())
    expect(url.searchParams.get('did')).toBe('did:bns:bob')
    expect(url.searchParams.get('redirect_uri')).toMatch(/\/sso\/did_callback$/)
  })
})

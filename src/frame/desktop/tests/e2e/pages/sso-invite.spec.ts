import { expect, test, type Page } from '@playwright/test'

/** The mock control panel (src/api/control_panel_mock.ts) seeds invites ei-active / ei-expired / ei-password. */
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

test.describe('exclusive invite page', () => {
  test('a different signed-in identity is never replaced silently', async ({ page }) => {
    await page.goto('/sso/invite/ei-active#s=mock-secret-active')
    const conflict = page.getByTestId('sso-invite-conflict')
    await expect(conflict).toContainText('You are signed in as Alice, but this link belongs to Acme Customer.')
    await expect(page.getByTestId('sso-invite-resource')).toHaveText('Ticket #123')
    // The secret left the address bar at once.
    expect(new URL(page.url()).hash).toBe('')

    await conflict.getByTestId('sso-invite-conflict-cancel').click()
    await expect(page.getByTestId('sso-invite-cancelled')).toContainText('you are still signed in as Alice')
    const state = await mockState(page)
    expect(state.session).toBeUndefined()
    expect(state.account.user_id).toBe('alice')
  })

  test('continue as the invited account lands in the restricted shell, which offers a first password', async ({ page }) => {
    await page.goto('/sso/invite/ei-active#s=mock-secret-active')
    await page.getByTestId('sso-invite-conflict-continue').click()
    await page.waitForURL('**/?invite=ei-active')

    const home = page.getByTestId('restricted-home')
    await expect(home.getByTestId('sso-identity-name')).toHaveText('Acme Customer')
    await expect(home.getByTestId('sso-identity-kind')).toHaveText('Restricted account')
    await expect(home.getByTestId('sso-identity-did')).toHaveText('did:bns:acme-c1')
    await expect(page.getByTestId('desktop-taskbar')).toHaveCount(0)

    const prompt = home.getByTestId('sso-first-password')
    await prompt.getByTestId('sso-first-password-open').click()
    await prompt.getByLabel('New password').fill('short')
    await prompt.getByLabel('Confirm password').fill('short')
    await prompt.getByTestId('sso-first-password-save').click()
    await expect(prompt).toContainText('Use at least 8 characters.')
    await prompt.getByLabel('New password').fill('a-long-password')
    await prompt.getByLabel('Confirm password').fill('a-long-password')
    await prompt.getByTestId('sso-first-password-save').click()
    await expect(home.getByTestId('sso-first-password-done')).toContainText('sign in as acme-c1')

    const state = await mockState(page)
    expect(state.sso.accounts['acme-c1'].password_set).toBe(true)
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    expect(state.sso.invites.find((invite: any) => invite.invite_id === 'ei-active').credential.state).toBe('consumed_by_password')
  })

  test('an expired link asks for a resend, never shows a link, and is rate limited', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto('/sso/invite/ei-expired#s=mock-secret-expired')
    const denied = page.getByTestId('sso-invite-denied')
    await expect(denied).toHaveAttribute('data-detail', 'expired')
    await expect(denied.getByTestId('sso-denial')).toHaveText('This link has expired.')
    await denied.getByTestId('sso-invite-resend').click()
    await expect(denied.getByTestId('sso-invite-resend-result')).toHaveAttribute('data-status', 'submitted')
    await expect(denied.getByTestId('sso-invite-resend')).toHaveCount(0)
    await expect(denied).not.toContainText('/sso/invite/')

    // A fresh page load: the address without the fragment is the same document.
    await page.goto('about:blank')
    await page.goto('/sso/invite/ei-expired#s=mock-secret-expired')
    await page.getByTestId('sso-invite-resend').click()
    await expect(page.getByTestId('sso-invite-resend-result')).toHaveAttribute('data-status', 'rate_limited')
  })

  test('an account with a password signs in with it; the link prefills the account', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto('/sso/invite/ei-password#s=mock-secret-password')
    const denied = page.getByTestId('sso-invite-denied')
    await expect(denied.getByTestId('sso-denial')).toHaveText('This account has a password now. Sign in with it.')
    await denied.getByTestId('sso-invite-password-login').click()
    await page.waitForURL('**/login?**')
    await expect(page.getByLabel('Username')).toHaveValue('acme-c3')
    expect(new URL(new URL(page.url()).searchParams.get('redirect_url') ?? '').search).toBe('?invite=ei-password')
  })

  test('a replaced or revoked link says so', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto('/sso/invite/ei-active#s=an-old-secret')
    await expect(page.getByTestId('sso-denial')).toHaveText('This link was replaced by a newer one. Use the latest link you received.')
    await page.goto('/sso/invite/ei-unknown#s=whatever')
    await expect(page.getByTestId('sso-invite-denied')).toHaveAttribute('data-reason', 'source_revoked')
    await page.goto('about:blank')
    await page.goto('/sso/invite/ei-active')
    await expect(page.getByTestId('sso-invite-no-secret')).toBeVisible()
  })

  test('an anonymous visitor enters as the invited account directly', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto('/sso/invite/ei-active#s=mock-secret-active')
    await page.waitForURL('**/?invite=ei-active')
    await expect(page.getByTestId('restricted-home').getByTestId('sso-identity-name')).toHaveText('Acme Customer')
    expect((await mockState(page)).session).toEqual({ kind: 'account', auth_method: 'exclusive_invite' })
  })
})

test.describe('restricted shell', () => {
  test('an External DID gets the restricted shell, not the Desktop', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'external', did: 'did:bns:carol.other', show_name: 'Carol' } })
    await page.goto('/')
    const home = page.getByTestId('restricted-home')
    await expect(home.getByTestId('sso-identity-kind')).toHaveText('External identity')
    await expect(home.getByTestId('sso-identity-did')).toHaveText('did:bns:carol.other')
    await expect(home.getByTestId('sso-first-password')).toHaveCount(0)
    await expect(home.getByTestId('restricted-home-hint')).toContainText('identity from another Zone')
    await expect(page.getByTestId('desktop-taskbar')).toHaveCount(0)
  })

  test('without a session `/` goes to sign in', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto('/')
    await page.waitForURL('**/login?**')
  })
})

import { expect, test, type Page } from '@playwright/test'

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

test.describe('login page', () => {
  test('has no remember-me option and signs in', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto('/login')
    await expect(page.getByRole('button', { name: 'Sign In', exact: true })).toBeVisible()
    await expect(page.getByText(/remember me/i)).toHaveCount(0)
    await expect(page.getByRole('checkbox')).toHaveCount(0)

    await page.getByLabel('Username').fill('bob')
    await page.getByLabel('Password').fill('a-password')
    await page.getByRole('button', { name: 'Sign In', exact: true }).click()
    await page.waitForURL((url) => url.pathname === '/')
    const state = await mockState(page)
    expect(state.account.user_id).toBe('bob')
    expect(state.session).toEqual({ kind: 'account', auth_method: 'password' })
  })

  test('a denial is shown by its reason', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto('/login')
    await page.getByLabel('Username').fill('acme-c1')
    await page.getByLabel('Password').fill('a-password')
    await page.getByRole('button', { name: 'Sign In', exact: true }).click()
    await expect(page.getByTestId('login-error')).toHaveText('This account has no password yet. Open the invitation link you received to sign in.')
  })

  test('shows the DID callback error and offers DID login only when the Zone allows it', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto(`/login?did_error=canonical_unavailable&redirect_url=${encodeURIComponent('http://127.0.0.1:4173/sso/entry?service=aiworkspace&entry=se-dj&join=1')}`)
    await expect(page.getByTestId('login-did-error')).toHaveText('The home Zone of your DID could not be reached. Try again later.')
    await page.getByTestId('login-use-did').click()
    await expect(page.getByTestId('sso-did-form')).toBeVisible()

    await page.goto('/login?did_error=zone_method_disabled')
    await expect(page.getByTestId('login-did-error')).toHaveText('This sign-in method is not available right now. Contact the administrator.')
  })

  test('hides DID login when the Zone switched it off', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' }, sso: { settings: { self_register_enabled: true, did_login_enabled: false } } })
    await page.goto('/login?username=acme-c3')
    await expect(page.getByLabel('Username')).toHaveValue('acme-c3')
    await expect(page.getByRole('button', { name: 'Sign In', exact: true })).toBeVisible()
    // The Zone switches were read (the mock keeps its SSO state once asked).
    await expect.poll(async () => Boolean((await mockState(page)).sso?.entries)).toBe(true)
    await expect(page.getByTestId('login-use-did')).toHaveCount(0)
  })
})

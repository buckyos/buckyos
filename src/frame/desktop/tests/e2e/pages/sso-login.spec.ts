import { expect, test, type Page } from '@playwright/test'
import { MOCK_APP_LAUNCH_KEY, routeAppHost } from '../fixtures/desktop'

const MOCK_CONTROL_PANEL = 'buckyos.mock.control-panel.v1'
const homeConsent = 'https://bob.web3.buckyos.ai/sso/federate'

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
  test('custom domain requester is displayed as Desktop, not a hostname label', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto(`/login?redirect_url=${encodeURIComponent('https://streetycat-ood.cc.cd/')}`)
    await expect(page.getByText('Requested by Desktop')).toBeVisible()
    await expect(page.getByText('Requested by streetycat-ood')).toHaveCount(0)
  })

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

  test('a workspace DID login resumes on the Zone root, including after cancellation, then opens the original page', async ({ page, baseURL }) => {
    const login = new URL('/login', baseURL)
    login.hostname = 'localhost'
    const workspace = new URL('/design/draft?id=s1&entry=se-editor#section-3', login)
    workspace.hostname = 'aiws-2611abcd.localhost'
    login.searchParams.set('redirect_url', workspace.href)
    const callbacks = await routeAppHost(page, workspace.origin + '/**', '<p>Workspace</p>')
    await page.route(homeConsent + '?**', (route) => route.fulfill({ contentType: 'text/html', body: '<p>Home Zone</p>' }))
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto(login.href)

    async function beginDid() {
      await page.getByTestId('login-use-did').click()
      await page.getByTestId('sso-did-input').fill('did:bns:bob')
      await page.getByTestId('sso-did-submit').click()
      await page.waitForURL(homeConsent + '?**')
      const flow = new URL(page.url()).searchParams.get('state')!
      const { origins } = await page.context().storageState()
      const saved = origins.find((origin) => origin.origin === login.origin)!.localStorage.find(({ name }) => name === MOCK_CONTROL_PANEL)!
      return new URL(JSON.parse(saved.value).sso.did_logins[flow].redirect_url)
    }

    const resume = await beginDid()
    expect(resume.origin).toBe(login.origin)
    expect(resume.pathname).toBe('/login')
    expect(resume.searchParams.get('redirect_url')).toBe(workspace.href)

    // The real callback returns the reason and the very URL recorded by begin.
    const cancelled = new URL('/login', login)
    cancelled.searchParams.set('did_error', 'access_denied')
    cancelled.searchParams.set('redirect_url', resume.href)
    await page.goto(cancelled.href)
    await expect(page.getByTestId('login-did-error')).toContainText('cancelled')
    expect((await beginDid()).href).toBe(resume.href)

    // The successful callback has signed this tab in on the root before sending it to resume.
    await page.addInitScript((key) => {
      if (location.pathname !== '/login') return
      const state = JSON.parse(localStorage.getItem(key)!)
      state.session = { kind: 'external', did: 'did:bns:bob', show_name: 'Bob' }
      localStorage.setItem(key, JSON.stringify(state))
    }, MOCK_CONTROL_PANEL)
    await page.goto(resume.href)
    await expect(page).toHaveURL(workspace.href)
    expect(callbacks).toHaveLength(1)
    expect(new URL(callbacks[0]).searchParams.get('redirect_url')).toBe(workspace.href)
  })

  test('a failed workspace handoff leaves a visible error and the sign-in form', async ({ page, baseURL }) => {
    const login = new URL('/login', baseURL)
    login.hostname = 'localhost'
    const workspace = new URL('/design', login)
    workspace.hostname = 'aiws-2611abcd.localhost'
    login.searchParams.set('redirect_url', workspace.href)
    await seedControlPanel(page, { session: { kind: 'external', did: 'did:bns:bob', show_name: 'Bob' } })
    await page.addInitScript((key) => localStorage.setItem(key, 'access_denied'), MOCK_APP_LAUNCH_KEY)
    await page.goto(login.href)
    await expect(page.getByTestId('login-workspace-error')).toBeVisible()
    await expect(page.getByTestId('login-use-did')).toBeVisible()
    await expect(page).toHaveURL(login.href)
  })

  test('a workspace host in a different Zone cannot use this Zone’s handoff', async ({ page, baseURL }) => {
    const login = new URL('/login', baseURL)
    login.searchParams.set('redirect_url', 'https://aiws-2611abcd.other.example/design')
    await seedControlPanel(page, { session: { kind: 'external', did: 'did:bns:bob', show_name: 'Bob' } })
    await page.goto(login.href)
    await expect(page.getByTestId('login-use-did')).toBeVisible()
    expect(await page.evaluate(() => sessionStorage.getItem('buckyos.mock.app-launch.count'))).toBeNull()
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

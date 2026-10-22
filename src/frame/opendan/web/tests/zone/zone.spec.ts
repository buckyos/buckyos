import { expect, test, type Page } from '@playwright/test'

const user = process.env.OPENDAN_ZONE_USER ?? 'devtest'
const password = process.env.OPENDAN_ZONE_PASSWORD ?? 'bucky2025'

const signIn = async (page: Page) => {
  await page.locator('input[type="text"]').fill(user)
  await page.locator('input[type="password"]').fill(password)
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
}

test('a fresh Agent opens its home and Loader through the app API', async ({ page }) => {
  const errors: string[] = []
  const failures: string[] = []
  const apiMethods: string[] = []
  page.on('pageerror', (error) => errors.push(error.message))
  page.on('response', (response) => {
    if (response.status() === 503) failures.push(response.url())
    if (new URL(response.url()).pathname !== '/api/opendan') return
    apiMethods.push(response.request().method())
    if (response.status() !== 200) failures.push(`${response.status()} ${response.url()}`)
  })
  const metadata = page.waitForResponse((response) =>
    new URL(response.url()).pathname === '/api/opendan' && response.request().method() === 'GET',
  )
  await page.goto('/?lang=en')
  await expect(page).toHaveURL(/\/login\?/)
  await signIn(page)
  const { app_id: appId } = await (await metadata).json() as { app_id: string }
  expect(appId).toBeTruthy()
  await expect(page.getByTestId('data-source')).toHaveText('data: krpc')
  await expect(page.getByTestId('agent-did')).toContainText(appId)
  await page.getByRole('link', { name: 'Advanced' }).click()
  await page.getByRole('link', { name: 'Loader', exact: true }).click()
  await expect(page.getByTestId('loader-info')).toContainText(appId)
  await expect(page.getByTestId('loader-modules')).toContainText('agent_state_service')
  await expect(page.getByRole('alert')).toHaveCount(0)
  expect(apiMethods).toEqual(expect.arrayContaining(['GET', 'POST']))
  expect(failures).toEqual([])
  expect(errors).toEqual([])
})

test('the WebUI behind the zone gateway reads the Loader with the zone login', async ({ page }) => {
  const errors: string[] = []
  const denied: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  page.on('response', async (r) => {
    if (!r.url().endsWith('/api/opendan')) return
    const body = await r.text().catch(() => '')
    if (body.includes('No permission')) denied.push(`${r.request().postDataJSON()?.method}: ${body.slice(0, 160)}`)
  })

  await page.goto('/')
  await expect(page).toHaveURL(/\/login\?/)
  await signIn(page)

  await expect(page.getByTestId('data-source')).toHaveText('data: krpc')
  await expect(page.getByTestId('agent-card')).toBeVisible()
  await expect(page.getByTestId('session-list').locator('a[data-target="messagehub"]').first()).toBeVisible()
  await page.screenshot({ path: 'test-results-zone/home.png', fullPage: true })
  await page.getByRole('link', { name: 'Advanced' }).click()
  await expect(page.getByTestId('sessions-panel')).toBeVisible()
  await expect(page.getByTestId('sessions-panel')).toContainText('ui-')
  await page.screenshot({ path: 'test-results-zone/sessions.png', fullPage: true })

  await page.getByTestId('sessions-panel').getByRole('link').filter({ hasText: 'ui-' }).first().click()
  await expect(page.locator('body')).toContainText('turn_ended')
  await page.screenshot({ path: 'test-results-zone/session.png', fullPage: true })

  await page.getByRole('link', { name: 'Loader' }).click()
  await expect(page.getByTestId('loader-info')).toContainText('did:')
  await expect(page.getByTestId('loader-modules')).toContainText('ui')
  await page.screenshot({ path: 'test-results-zone/loader.png', fullPage: true })

  expect(denied).toEqual([])
  expect(errors).toEqual([])
})

// What a restart of verify-hub leaves behind: the page still has its session
// cookie, but the zone no longer knows the refresh token, so renewing fails and
// the zone clears both cookies. A token it cannot parse takes the same path.
test('a login the zone no longer accepts asks to sign in again', async ({ page, context, baseURL }) => {
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  await page.goto('/')
  await signIn(page)
  await expect(page.getByTestId('agent-card')).toBeVisible()

  await context.clearCookies({ name: 'buckyos_refresh_token' })
  await context.addCookies([
    { name: 'buckyos_refresh_token', value: 'forgotten', url: `${baseURL}/`, httpOnly: true, secure: true, sameSite: 'Lax' },
  ])
  await page.reload()
  const card = page.getByTestId('login-lost')
  await expect(card).toBeVisible()
  await expect(page.locator('body')).not.toContainText('transport')
  await page.screenshot({ path: 'test-results-zone/login-lost.png', fullPage: true })

  await card.getByRole('button').click()
  await expect(page).toHaveURL(/\/login\?/)
  await signIn(page)
  await expect(page.getByTestId('agent-card')).toBeVisible()
  await expect(page.getByTestId('login-lost')).toHaveCount(0)
  expect(errors).toEqual([])
})

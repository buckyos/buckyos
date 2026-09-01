import { createHash, randomBytes } from 'node:crypto'
import { expect, test, type APIRequestContext, type Page, type TestInfo } from '@playwright/test'

type JsonRecord = Record<string, unknown>

const adminUser = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const adminPassword = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || ''

function hashPassword(username: string, password: string, nonce?: number): string {
  const original = createHash('sha256')
    .update(`${password}${username}.buckyos`, 'utf8')
    .digest('base64')
  if (nonce === undefined) return original
  return createHash('sha256').update(`${original}${nonce}`, 'utf8').digest('base64')
}

async function rpc(
  request: APIRequestContext,
  baseURL: string,
  service: string,
  method: string,
  params: JsonRecord,
  token?: string,
): Promise<JsonRecord> {
  const seq = Date.now()
  const response = await request.post(`${baseURL}/kapi/${service}`, {
    data: { method, params, sys: token ? [seq, token] : [seq] },
    ignoreHTTPSErrors: true,
  })
  if (!response.ok()) throw new Error(`${method} HTTP ${response.status()}`)
  const body = await response.json() as JsonRecord
  if (body.error) throw new Error(String(body.error))
  const result = body.result
  if (!result || typeof result !== 'object' || Array.isArray(result)) {
    throw new Error(`${method} returned an invalid result`)
  }
  return result as JsonRecord
}

async function apiLogin(
  request: APIRequestContext,
  baseURL: string,
  username: string,
  password: string,
): Promise<JsonRecord> {
  const nonce = Date.now()
  return rpc(request, baseURL, 'control-panel', 'auth.login', {
    username,
    password: hashPassword(username, password, nonce),
    appid: 'control-panel',
    target: { kind: 'system', service_id: 'control-panel' },
    login_nonce: nonce,
  })
}

async function apiSudo(
  request: APIRequestContext,
  baseURL: string,
  username: string,
  password: string,
): Promise<string> {
  const nonce = Date.now() + 1
  const result = await rpc(request, baseURL, 'verify-hub', 'sudo_by_password', {
    username,
    password: hashPassword(username, password, nonce),
    target: { kind: 'system', service_id: 'control-panel' },
    aud: 'system-config',
    login_nonce: nonce,
  })
  if (typeof result.session_token !== 'string' || !result.session_token) {
    throw new Error('sudo_by_password did not return a token')
  }
  return result.session_token
}

async function cleanupUser(
  request: APIRequestContext,
  baseURL: string,
  userId: string,
): Promise<void> {
  const login = await apiLogin(request, baseURL, adminUser, adminPassword)
  if (typeof login.session_token !== 'string') throw new Error('admin login returned no token')
  const sudoToken = await apiSudo(request, baseURL, adminUser, adminPassword)
  await rpc(request, baseURL, 'control-panel', 'user.delete', { user_id: userId }, sudoToken)
}

async function loginThroughUi(page: Page, username: string, password: string): Promise<void> {
  await page.goto('/')
  await expect(page.getByLabel('Username')).toBeVisible()
  await page.getByLabel('Username').fill(username)
  await page.getByLabel('Password').fill(password)
  await page.getByRole('button', { name: 'Sign In' }).click()
  await expect(page.getByRole('button', { name: 'BuckyOS' })).toBeVisible()
}

function jwtPayload(token: string): JsonRecord {
  const segments = token.split('.')
  if (segments.length !== 3) throw new Error('session token is not a JWT')
  return JSON.parse(Buffer.from(segments[1], 'base64url').toString('utf8')) as JsonRecord
}

async function refreshBrowserSso(page: Page, expectedUserId: string): Promise<void> {
  const before = await page.context().cookies()
  const previousRefresh = before.find((cookie) => cookie.name === 'buckyos_refresh_token')?.value
  expect(previousRefresh, 'refresh cookie before rotation').toBeTruthy()
  await page.context().clearCookies({ name: 'buckyos_session_token' })
  expect(
    (await page.context().cookies()).some((cookie) => cookie.name === 'buckyos_session_token'),
    'expired session cookie is absent before refresh',
  ).toBe(false)

  const result = await page.evaluate(async () => {
    const response = await fetch('/sso_refresh', {
      method: 'POST',
      credentials: 'include',
      cache: 'no-store',
    })
    return {
      status: response.status,
      body: await response.json() as Record<string, unknown>,
    }
  })
  expect(result.status).toBe(200)
  expect(result.body.user_info).toMatchObject({ user_id: expectedUserId })
  const sessionToken = result.body.session_token
  expect(typeof sessionToken).toBe('string')
  const claims = jwtPayload(String(sessionToken))
  expect(claims).toMatchObject({
    iss: 'verify-hub',
    sub: expectedUserId,
    appid: 'control-panel',
    principal_kind: 'user',
    token_use: 'session',
    target_kind: 'system',
  })
  expect(claims.app_instance_id).toBeUndefined()
  expect(claims.app_owner_user_id).toBeUndefined()

  const after = await page.context().cookies()
  const rotatedRefresh = after.find((cookie) => cookie.name === 'buckyos_refresh_token')?.value
  expect(rotatedRefresh, 'refresh cookie after rotation').toBeTruthy()
  expect(rotatedRefresh).not.toBe(previousRefresh)
}

async function openUsersAgents(page: Page) {
  await page.getByRole('button', { name: 'BuckyOS' }).click()
  await page.getByRole('complementary').getByRole('button', {
    name: 'Users & Agents',
    exact: true,
  }).click()
  const appWindow = page.getByTestId('window-users-agents')
  await expect(appWindow).toBeVisible()
  await expect(appWindow.getByText('Internal Entities')).toBeVisible()
  return appWindow
}

async function attachFailureEvidence(
  page: Page,
  testInfo: TestInfo,
  consoleMessages: string[],
  lastRpcError: JsonRecord | null,
): Promise<void> {
  const screenshotPath = testInfo.outputPath('local-user-failure.png')
  await page.screenshot({ path: screenshotPath, fullPage: true })
  await testInfo.attach('failure-screenshot', { path: screenshotPath, contentType: 'image/png' })
  await testInfo.attach('console-errors', {
    body: Buffer.from(JSON.stringify(consoleMessages, null, 2)),
    contentType: 'application/json',
  })
  await testInfo.attach('last-rpc-error', {
    body: Buffer.from(JSON.stringify(lastRpcError, null, 2)),
    contentType: 'application/json',
  })
  await testInfo.attach('service-log-location', {
    body: Buffer.from('/opt/buckyos/logs'),
    contentType: 'text/plain',
  })
}

test('SSO callback rejects an alternate control-panel origin and consumes the nonce', async ({ request, baseURL }) => {
  test.skip(!adminPassword, 'Set BUCKYOS_TEST_ADMIN_PASSWORD for the real-zone UI DV.')
  if (!baseURL) throw new Error('Playwright baseURL is required')

  const loginNonce = Date.now()
  const canonicalRedirect = new URL('/desktop?source=callback-binding', baseURL).toString()
  const login = await rpc(request, baseURL, 'control-panel', 'auth.login', {
    username: adminUser,
    password: hashPassword(adminUser, adminPassword, loginNonce),
    appid: 'control-panel',
    redirect_url: canonicalRedirect,
    login_nonce: loginNonce,
  })
  expect(typeof login.sso_nonce).toBe('number')

  const alternateOrigin = new URL(baseURL)
  alternateOrigin.hostname = `sys.${alternateOrigin.hostname}`
  alternateOrigin.pathname = '/desktop'
  const callback = new URL('/sso_callback', baseURL)
  callback.searchParams.set('nonce', String(login.sso_nonce))
  callback.searchParams.set('redirect_url', alternateOrigin.toString())
  const rejected = await request.get(callback.toString(), {
    ignoreHTTPSErrors: true,
    maxRedirects: 0,
  })
  expect(rejected.status()).toBeGreaterThanOrEqual(400)

  callback.searchParams.set('redirect_url', canonicalRedirect)
  const replay = await request.get(callback.toString(), {
    ignoreHTTPSErrors: true,
    maxRedirects: 0,
  })
  expect(replay.status()).toBeGreaterThanOrEqual(400)
})

test('admin creates a real local user, then that user logs in and sees self', async ({ page, request, baseURL }, testInfo) => {
  test.skip(!adminPassword, 'Set BUCKYOS_TEST_ADMIN_PASSWORD for the real-zone UI DV.')
  if (!baseURL) throw new Error('Playwright baseURL is required')

  const suffix = `${Date.now()}${randomBytes(2).toString('hex')}`
  const userId = `dvlocalui${suffix}`.slice(0, 48).toLowerCase()
  const displayName = `DV Local UI ${suffix.slice(-6)}`
  const localPassword = `Dv-${randomBytes(18).toString('hex')}!`
  const consoleErrors: string[] = []
  const rpcErrors: JsonRecord[] = []
  const responseTasks: Promise<void>[] = []
  let createAttempted = false

  page.on('console', (message) => {
    if (message.type() !== 'error') return
    const location = message.location().url
    const isExpectedAnonymousRefresh = location.endsWith('/sso_refresh')
      && message.text().includes('401 (Unauthorized)')
    if (!isExpectedAnonymousRefresh) {
      consoleErrors.push(location ? `${message.text()} (${location})` : message.text())
    }
  })
  page.on('request', (outgoing) => {
    if (outgoing.url().includes('/kapi/control-panel') && outgoing.postData()?.includes('"method":"user.create"')) {
      createAttempted = true
    }
  })
  page.on('response', (response) => {
    if (!response.url().includes('/kapi/')) return
    responseTasks.push((async () => {
      try {
        const body = await response.json() as JsonRecord
        if (body.error) rpcErrors.push({ url: response.url(), error: body.error })
      } catch {
        if (response.status() >= 400) rpcErrors.push({ url: response.url(), status: response.status() })
      }
    })())
  })

  try {
    await loginThroughUi(page, adminUser, adminPassword)
    await refreshBrowserSso(page, adminUser)
    await page.reload()
    await expect(page.getByRole('button', { name: 'BuckyOS' })).toBeVisible()
    let appWindow = await openUsersAgents(page)
    await appWindow.getByLabel('Add User').click()
    await appWindow.getByLabel('Local username').fill(userId)
    await appWindow.getByLabel('Display name').fill(displayName)
    await appWindow.getByLabel('Initial password').fill(localPassword)
    await appWindow.getByLabel('Confirm password').fill(localPassword)
    await appWindow.getByRole('button', { name: 'Next' }).click()
    await expect(appWindow.getByText('No apps are installed or promised')).toBeVisible()
    await appWindow.getByRole('button', { name: 'Create User', exact: true }).click()

    const sudoDialog = page.getByRole('dialog', { name: 'Create local user' })
    await expect(sudoDialog).toBeVisible()
    await sudoDialog.locator('input[type="password"]').fill(adminPassword)
    await sudoDialog.getByRole('button', { name: 'Create user', exact: true }).click()

    await expect(appWindow.getByRole('heading', { name: displayName })).toBeVisible()
    await expect(appWindow.getByText('Local account · zone-members')).toBeVisible()
    await expect(appWindow.getByText('Local', { exact: true })).toBeVisible()
    await expect(appWindow.locator('.MuiChip-label').filter({ hasText: /^user$/ })).toBeVisible()
    await expect(appWindow.locator('.MuiChip-label').filter({ hasText: /^active$/ })).toBeVisible()
    await page.screenshot({ path: testInfo.outputPath('admin-created-user.png'), fullPage: true })

    await page.getByRole('button', { name: 'BuckyOS' }).click()
    await page.getByRole('button', { name: 'Log out' }).click()
    await expect(page.getByLabel('Username')).toBeVisible()
    const cookiesAfterLogout = await page.context().cookies()
    expect(
      cookiesAfterLogout.filter((cookie) =>
        cookie.name === 'buckyos_session_token' || cookie.name === 'buckyos_refresh_token'
      ),
      'logout clears both SSO cookies',
    ).toEqual([])

    await loginThroughUi(page, userId, localPassword)
    await refreshBrowserSso(page, userId)
    await page.reload()
    await expect(page.getByRole('button', { name: 'BuckyOS' })).toBeVisible()
    appWindow = await openUsersAgents(page)
    await expect(appWindow.getByText('Internal Entities')).toBeVisible()
    await appWindow.getByRole('button', { name: new RegExp(displayName) }).click()
    await expect(appWindow.getByRole('heading', { name: displayName })).toBeVisible()
    await expect(appWindow.locator('button[aria-current="page"]')).toHaveCount(1)
    const listedEntityIds = await appWindow.locator('button[data-entity-id]').evaluateAll(
      (items) => items.map((item) => item.getAttribute('data-entity-id')),
    )
    expect(new Set(listedEntityIds).size, 'internal entity ids must be unique').toBe(listedEntityIds.length)
    await expect(appWindow.getByText(`did:web:${userId}.test.buckyos.io`, { exact: true })).toBeVisible()
    await expect(appWindow.getByRole('heading', { name: 'Profile' })).toBeVisible()
    await expect(appWindow.getByRole('heading', { name: 'Settings' })).toBeVisible()
    await expect(appWindow.getByText('Change allowed', { exact: true })).toBeVisible()
    await page.screenshot({ path: testInfo.outputPath('local-user-self.png'), fullPage: true })

    await Promise.all(responseTasks)
    expect(consoleErrors, 'browser console errors').toEqual([])
    expect(rpcErrors, 'Gateway KRPC errors or HTTP failures').toEqual([])
  } catch (error) {
    await Promise.allSettled(responseTasks)
    await attachFailureEvidence(page, testInfo, consoleErrors, rpcErrors.at(-1) ?? null)
    throw error
  } finally {
    if (createAttempted) {
      try {
        await cleanupUser(request, baseURL, userId)
      } catch (error) {
        await testInfo.attach('cleanup-error', {
          body: Buffer.from(error instanceof Error ? error.stack ?? error.message : String(error)),
          contentType: 'text/plain',
        })
      }
    }
  }
})

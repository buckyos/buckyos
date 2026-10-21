/**
 * Opening a web App from the desktop without a password, on a real Zone
 * (notepads/desktop-app-sso-auto-login-todo.md):
 *
 *   BUCKYOS_TEST_ZONE_HOST=test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=… \
 *     pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/app-launch.spec.ts
 *
 * Needs an App instance the owner (`devtest`) may use (`BUCKYOS_TEST_LAUNCH_APP`, default the
 * preinstalled text editor) and one owned by someone else that the owner may not use
 * (`BUCKYOS_TEST_DENIED_APP`). Signs in through the real login page at the Zone root, writes one
 * Markdown file into the owner's home through NFSP and deletes it again.
 */
import { createHash } from 'node:crypto'
import { expect, test, type APIRequestContext, type Browser, type BrowserContext, type Frame, type Page } from '@playwright/test'
import { openFromAllApps } from '../e2e/fixtures/desktop'

type JsonRecord = Record<string, unknown>
type NfsResult = { session: string; ref: unknown; fb_handle: string; lease: { lease_id: string } }

const zone = process.env.BUCKYOS_TEST_ZONE_HOST || 'test.buckyos.io'
const origin = `https://${zone}`
const owner = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const ownerPassword = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025'
const peer = process.env.BUCKYOS_TEST_SHARE_USER || 'bob'
const peerPassword = process.env.BUCKYOS_TEST_USER_PASSWORD || 'buckyos2026'
const launchApp = process.env.BUCKYOS_TEST_LAUNCH_APP || `text-editor.buckyos.bns.did@${owner}`
const launchHost = process.env.BUCKYOS_TEST_LAUNCH_HOST || 'text-editor'
const deniedApp = process.env.BUCKYOS_TEST_DENIED_APP || `r03share.test.buckyos.io@${peer}`
const deniedHost = process.env.BUCKYOS_TEST_DENIED_HOST || `r03share-${peer}`
const appOrigin = `https://${launchHost}.${zone}`

function hashPassword(username: string, password: string, nonce: number): string {
  const original = createHash('sha256').update(`${password}${username}.buckyos`, 'utf8').digest('base64')
  return createHash('sha256').update(`${original}${nonce}`, 'utf8').digest('base64')
}

function claims(jwt: string): JsonRecord {
  return JSON.parse(Buffer.from(jwt.split('.')[1], 'base64url').toString('utf8')) as JsonRecord
}

async function call(request: APIRequestContext, service: string, method: string, params: JsonRecord, token?: string) {
  const seq = Date.now()
  const response = await request.post(`${origin}/kapi/${service}`, { data: { method, params, sys: token ? [seq, token] : [seq] } })
  expect(response.status(), `${method} HTTP status`).toBe(200)
  return { body: await response.json() as { result?: JsonRecord; error?: string }, headers: response.headers() }
}

async function apiLogin(request: APIRequestContext, username: string, password: string) {
  const nonce = Date.now()
  const { body } = await call(request, 'control-panel', 'auth.login', {
    username,
    password: hashPassword(username, password, nonce),
    target: { kind: 'system', service_id: 'control-panel' },
    login_nonce: nonce,
  })
  if (!body.result) throw new Error(`auth.login ${username}: ${body.error}`)
  return { token: String(body.result.session_token), refresh: String(body.result.refresh_token) }
}

const prepare = (request: APIRequestContext, token: string, appInstanceId: string, redirectUrl: string, extra: JsonRecord = {}) =>
  call(request, 'control-panel', 'auth.prepare_app_launch', { app_instance_id: appInstanceId, redirect_url: redirectUrl, ...extra }, token)

async function uiLogin(browser: Browser, username: string, password: string): Promise<{ context: BrowserContext; page: Page }> {
  const context = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1440, height: 1000 } })
  const page = await context.newPage()
  await page.goto(`${origin}/`)
  await page.getByLabel('Username', { exact: true }).fill(username)
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  await expect(page.getByRole('button', { name: 'BuckyOS' }).first()).toBeVisible({ timeout: 30_000 })
  return { context, page }
}

/** The SSO cookies written for the App host itself (host-only, so their domain is exactly that host). */
async function appCookies(context: BrowserContext) {
  const cookies = (await context.cookies(appOrigin)).filter((cookie) => cookie.domain === `${launchHost}.${zone}`)
  const session = cookies.find((cookie) => cookie.name === 'buckyos_session_token')
  const refresh = cookies.find((cookie) => cookie.name === 'buckyos_refresh_token')
  return { session, refresh }
}

async function landedFrame(page: Page): Promise<Frame> {
  await expect.poll(() => page.frames().find((frame) => frame.url().startsWith(`${appOrigin}/`) && !frame.url().includes('/sso_callback'))?.url() ?? '', { timeout: 30_000 })
    .toMatch(new RegExp(`^${appOrigin.replace(/\./g, '\\.')}/.*#bfp=`))
  return page.frames().find((frame) => frame.url().startsWith(`${appOrigin}/`))!
}

test('the desktop opens an App signed in, without a password page', async ({ browser }) => {
  const { context, page } = await uiLogin(browser, owner, ownerPassword)
  const refreshes: number[] = []
  const refreshCookieHeaders: string[] = []
  page.on('response', (response) => {
    if (response.url() === `${appOrigin}/sso_refresh`) refreshes.push(response.status())
  })
  page.on('request', (request) => {
    if (request.url() === `${appOrigin}/sso_refresh`) void request.headerValue('cookie').then((value) => refreshCookieHeaders.push(value ?? ''))
  })
  const loginPages: string[] = []
  page.on('framenavigated', (frame) => {
    if (frame !== page.mainFrame() && frame.url().startsWith('http') && new URL(frame.url()).pathname === '/login') loginPages.push(frame.url())
  })

  await openFromAllApps(page, launchApp)
  const iframe = page.getByTestId('web-app-frame')
  await expect(iframe).toHaveAttribute('src', new RegExp(`^${appOrigin.replace(/\./g, '\\.')}/sso_callback\\?nonce=\\d+&redirect_url=`))

  await landedFrame(page)
  await expect.poll(() => refreshes.filter((status) => status === 200).length, { timeout: 30_000 }).toBeGreaterThan(0)
  expect(loginPages).toEqual([])

  // Only the App host's own pair travels with its requests: the Zone root's host-only cookies stay there.
  expect(refreshCookieHeaders.length).toBeGreaterThan(0)
  for (const header of refreshCookieHeaders) expect(header.match(/buckyos_refresh_token=/g)?.length).toBe(1)
  const { session, refresh } = await appCookies(context)
  expect(session).toBeTruthy()
  expect(refresh?.httpOnly).toBe(true)
  const sessionClaims = claims(session!.value)
  expect(sessionClaims).toMatchObject({ sub: owner, target_kind: 'app', app_instance_id: launchApp, principal_kind: 'user', token_use: 'session' })
  expect(sessionClaims.sudo ?? false).toBe(false)
  await context.close()
})

test('a content open lands on the full App URL and the AppFrame handshake completes', async ({ browser, request }) => {
  const { token } = await apiLogin(request, owner, ownerPassword)
  const name = `app-launch-dv-${Date.now().toString(36)}.md`
  const nfs = async (method: string, body: JsonRecord) => {
    const response = await request.post(`${origin}/nfs/v1/${method}`, { headers: { Authorization: `Bearer ${token}` }, data: body })
    const parsed = await response.json() as { ok: boolean; result?: JsonRecord; error?: JsonRecord }
    if (!parsed.ok) throw new Error(`${method}: ${JSON.stringify(parsed.error)}`)
    return parsed.result as NfsResult
  }
  const session = (await nfs('hello', { args: {} })).session
  const home = await nfs('stat', { session, args: {}, at: { path: `/home/${owner}` } })
  const bytes = Buffer.from('# App launch DV\n\nopened from the desktop\n')
  const open = await nfs('open_write', { session, seq: 1, args: { parent_ref: home.ref, name, size: bytes.length } })
  const patch = await request.fetch(`${origin}/nfs/v1/uploads/${open.fb_handle}`, {
    method: 'PATCH',
    headers: { Authorization: `Bearer ${token}`, 'Upload-Offset': '0', 'Tus-Resumable': '1.0.0', 'Content-Type': 'application/offset+octet-stream' },
    data: bytes,
  })
  expect(patch.status()).toBe(204)
  await nfs('commit_file', { session, seq: 2, args: { name, fb_handle: open.fb_handle, lease_id: open.lease.lease_id }, at: { ref: home.ref } })

  const { context, page } = await uiLogin(browser, owner, ownerPassword)
  try {
    await page.goto(`${origin}/?open=${encodeURIComponent(`cyfs:///home/${owner}/${name}`)}`)
    const iframe = page.getByTestId('web-app-frame')
    await expect(iframe).toHaveAttribute('src', /\/sso_callback\?nonce=\d+&redirect_url=.*%23bfp%3D/)
    const frame = await landedFrame(page)
    const landed = new URL(frame.url())
    expect(landed.search).toContain(encodeURIComponent(name).slice(0, 12))
    // The editor reports its document title over the AppFrame protocol once it has the file.
    await expect(page.locator('[data-testid^="window-"]').filter({ hasText: name }).first()).toBeVisible({ timeout: 30_000 })
  } finally {
    await context.close()
    await nfs('delete', { session, seq: 3, args: { name }, at: { path: `/home/${owner}` } }).catch(() => {})
  }
})

test('two windows of one App keep their sign-in through concurrent refreshes', async ({ browser, request }) => {
  const { token } = await apiLogin(request, owner, ownerPassword)
  const context = await browser.newContext({ ignoreHTTPSErrors: true })
  const pages: Page[] = []
  for (let i = 0; i < 2; i++) {
    const { body } = await prepare(request, token, launchApp, `${appOrigin}/#bfp=dv-${i}`)
    expect(body.result?.status).toBe('ready')
    const page = await context.newPage()
    await page.goto(String(body.result!.callback_url))
    await expect(page).toHaveURL(new RegExp(`^${appOrigin.replace(/\./g, '\\.')}/#bfp=dv-${i}$`))
    pages.push(page)
  }
  const refreshAll = () => Promise.all(pages.map((page) => page.evaluate(async () => {
    const response = await fetch('/sso_refresh', { method: 'POST', credentials: 'include', cache: 'no-store' })
    return { status: response.status, token: response.ok ? (await response.json() as { session_token: string }).session_token : '' }
  })))
  for (let round = 0; round < 3; round++) {
    const results = await refreshAll()
    expect(results.map((result) => result.status)).toEqual([200, 200])
    expect(results[0].token).toBe(results[1].token)
  }
  const { refresh } = await appCookies(context)
  expect(refresh?.value).toBeTruthy()
  await context.close()
})

test('prepare_app_launch answers expected failures as statuses and stays protected', async ({ request }) => {
  const { token, refresh } = await apiLogin(request, owner, ownerPassword)

  const ready = await prepare(request, token, launchApp, `${appOrigin}/doc?x=1#bfp=dv`)
  expect(ready.body.result?.status).toBe('ready')
  expect(ready.headers['cache-control']).toBe('no-store')
  const callback = new URL(String(ready.body.result!.callback_url))
  expect(callback.origin).toBe(appOrigin)
  expect(callback.pathname).toBe('/sso_callback')
  expect(callback.searchParams.get('redirect_url')).toBe(`${appOrigin}/doc?x=1#bfp=dv`)
  expect(JSON.stringify(ready.body.result)).not.toContain('eyJ')

  expect((await prepare(request, token, deniedApp, `https://${deniedHost}.${zone}/`)).body.result).toEqual({ status: 'access_denied' })
  for (const [instance, url] of [
    [launchApp.replace(/@.*$/, `@${peer}`), `${appOrigin}/`],
    [launchApp, `${origin}/`],
    [launchApp, `https://${launchHost}.outside.example/`],
    [launchApp, `https://unknown-app-${Date.now().toString(36)}.${zone}/`],
  ]) {
    expect((await prepare(request, token, instance, url)).body.result, `${instance} ${url}`).toEqual({ status: 'invalid_target' })
  }
  expect((await prepare(request, token, launchApp, `${appOrigin}/`, { username: peer })).body.error).toBeTruthy()
  expect((await prepare(request, 'not-a-token', launchApp, `${appOrigin}/`)).body.result).toEqual({ status: 'source_session_inactive' })

  // An App session cannot launch anything, and the browser cannot call verify-hub directly.
  const delivered = await request.get(callback.href, { maxRedirects: 0 })
  expect(delivered.status()).toBe(302)
  expect(delivered.headers()['cache-control']).toBe('no-store')
  expect(delivered.headers()['location']).toBe(`${appOrigin}/doc?x=1#bfp=dv`)
  const appSession = delivered.headersArray().find((header) => header.name.toLowerCase() === 'set-cookie' && header.value.startsWith('buckyos_session_token='))!.value.split(';')[0].split('=')[1]
  expect(claims(appSession)).toMatchObject({ sub: owner, app_instance_id: launchApp })
  expect((await request.get(callback.href, { maxRedirects: 0 })).status()).not.toBe(302)
  const fromApp = await prepare(request, appSession, launchApp, `${appOrigin}/`)
  expect(fromApp.body.result).toBeUndefined()
  expect(fromApp.body.error).toContain('Control Panel user session')
  expect((await prepare(request, '', launchApp, `${appOrigin}/`)).body.result).toEqual({ status: 'source_session_inactive' })
  const direct = await call(request, 'verify-hub', 'delegate_user_session', { source_session_token: token, target: { kind: 'app', app_instance_id: launchApp } }, token)
  expect(direct.body.result).toBeUndefined()
  expect(direct.body.error).toContain('only control-panel may delegate')

  // Logout ends the source: later launches are refused.
  const logout = await call(request, 'verify-hub', 'logout', { refresh_token: refresh })
  expect(logout.body.result).toBe(true)
  expect((await prepare(request, token, launchApp, `${appOrigin}/`)).body.result).toEqual({ status: 'source_session_inactive' })
})

test('another user launches the shared App as themselves', async ({ request, browser }) => {
  const { token } = await apiLogin(request, peer, peerPassword)
  const { body } = await prepare(request, token, launchApp, `${appOrigin}/`)
  test.skip(body.result?.status === 'access_denied', `${launchApp} is not shared with ${peer}`)
  expect(body.result?.status).toBe('ready')
  const context = await browser.newContext({ ignoreHTTPSErrors: true })
  const page = await context.newPage()
  await page.goto(String(body.result!.callback_url))
  const { session } = await appCookies(context)
  expect(claims(session!.value)).toMatchObject({ sub: peer, app_instance_id: launchApp, app_owner_user_id: owner })
  await context.close()
})

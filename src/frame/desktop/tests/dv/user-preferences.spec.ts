/**
 * The user's settings for a page on its own origin, on a real Zone (notepads/user-preferences-api-todo.md):
 *
 *   BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=… \
 *     pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/user-preferences.spec.ts
 *
 * An App's session (the preinstalled text editor, `BUCKYOS_TEST_LAUNCH_APP`) gets the appearance and the public
 * profile, a system page's session the private profile too; a workspace's own tab shows the account's theme and
 * follows it. Changes the owner's theme and sets it back.
 */
import { createHash } from 'node:crypto'
import { expect, request as playwrightRequest, type APIRequestContext } from '@playwright/test'
import type { WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { test, workspaceOrigin } from './aiworkspace.fixtures'

type JsonRecord = Record<string, unknown>

const zone = process.env.BUCKYOS_TEST_ZONE_HOST || 'test.buckyos.io'
const origin = `https://${zone}`
const owner = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const ownerPassword = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025'
const launchApp = process.env.BUCKYOS_TEST_LAUNCH_APP || `text-editor.buckyos.bns.did@${owner}`
const launchHost = process.env.BUCKYOS_TEST_LAUNCH_HOST || 'text-editor'
const appOrigin = `https://${launchHost}.${zone}`

function hashPassword(username: string, password: string, nonce: number): string {
  const original = createHash('sha256').update(`${password}${username}.buckyos`, 'utf8').digest('base64')
  return createHash('sha256').update(`${original}${nonce}`, 'utf8').digest('base64')
}

async function call(request: APIRequestContext, at: string, service: string, method: string, params: JsonRecord, token: string) {
  const seq = Date.now()
  const response = await request.post(`${at}/kapi/${service}`, { data: { method, params, sys: [seq, token] } })
  expect(response.status(), `${method} HTTP status`).toBe(200)
  return await response.json() as { result?: JsonRecord; error?: string }
}

async function apiLogin(request: APIRequestContext): Promise<string> {
  const nonce = Date.now()
  const body = await call(request, origin, 'control-panel', 'auth.login', {
    username: owner, password: hashPassword(owner, ownerPassword, nonce), target: { kind: 'system', service_id: 'control-panel' }, login_nonce: nonce,
  }, '')
  if (!body.result) throw new Error(`auth.login: ${body.error}`)
  return String(body.result.session_token)
}

async function desktopState(request: APIRequestContext, token: string, body: JsonRecord): Promise<JsonRecord> {
  const response = await request.post(`${origin}/api/desktop`, { headers: { 'X-Auth': token }, data: { session_id: 'default', state_key: 'appearance', ...body } })
  const payload = await response.json() as { ok?: boolean; data?: JsonRecord; error?: string }
  if (!payload.ok) throw new Error(`/api/desktop ${String(body.action)}: ${payload.error}`)
  return payload.data ?? {}
}

const setTheme = (request: APIRequestContext, token: string, theme: string) =>
  desktopState(request, token, { action: 'state.set', json_path: 'theme', value: theme })

async function storedTheme(request: APIRequestContext, token: string): Promise<string | undefined> {
  const { value } = await desktopState(request, token, { action: 'state.get' })
  return (value as JsonRecord | null)?.theme as string | undefined
}

test('an App gets the appearance and the public profile, a system page the private profile too', async ({ browser }) => {
  const request = await playwrightRequest.newContext({ ignoreHTTPSErrors: true })
  const token = await apiLogin(request)
  const theme = await storedTheme(request, token)

  const system = await call(request, origin, 'control-panel', 'user.preferences.get', {}, token)
  expect(system.error).toBeUndefined()
  const profile = await call(request, origin, 'control-panel', 'user.profile.get', {}, token)
  const localProfile = profile.result?.local_profile ?? null
  if (theme) expect(system.result?.theme).toBe(theme)
  if (localProfile) expect(system.result?.private_profile).toEqual(localProfile)
  for (const field of ['iconSize', 'window', 'wallpaper', 'mobileFeed']) expect(system.result).not.toHaveProperty(field)

  // the App's own session, handed over on its host the way the Desktop launches it
  const launch = await call(request, origin, 'control-panel', 'auth.prepare_app_launch', { app_instance_id: launchApp, redirect_url: `${appOrigin}/` }, token)
  expect(launch.result?.status, JSON.stringify(launch)).toBe('ready')
  const context = await browser.newContext({ ignoreHTTPSErrors: true })
  try {
    const page = await context.newPage()
    await page.goto(String(launch.result?.callback_url))
    await expect.poll(async () => (await context.cookies(appOrigin)).some((cookie) => cookie.name === 'buckyos_session_token' && cookie.domain === `${launchHost}.${zone}`)).toBe(true)
    const appToken = (await context.cookies(appOrigin)).find((cookie) => cookie.name === 'buckyos_session_token' && cookie.domain === `${launchHost}.${zone}`)!.value
    const claims = JSON.parse(Buffer.from(appToken.split('.')[1], 'base64url').toString('utf8')) as JsonRecord
    expect(claims).toMatchObject({ sub: owner, target_kind: 'app', app_instance_id: launchApp })

    const app = await call(request, appOrigin, 'control-panel', 'user.preferences.get', {}, appToken)
    expect(app.error).toBeUndefined()
    const { private_profile: _private, ...shared } = system.result ?? {}
    expect(app.result).toEqual(shared)
    expect(app.result).not.toHaveProperty('private_profile')
    // the gate cannot be walked around: the App's session reads no profile of its own accord
    const direct = await call(request, appOrigin, 'control-panel', 'user.profile.get', {}, appToken)
    expect(direct.result, JSON.stringify(direct)).toBeUndefined()
  } finally {
    await context.close()
    await request.dispose()
  }
})

test("a workspace's own tab shows the account's theme and follows it", async ({ owner: actor }) => {
  const request = await playwrightRequest.newContext({ ignoreHTTPSErrors: true })
  const token = await actor.client.session()
  const before = await storedTheme(request, token)
  try {
    // made through the service: the Desktop speaks the account's language, which this test does not pin
    const created = await actor.client.ok<WorkspaceSummary>('ws.create', { title: actor.client.prefix })
    actor.client.owned.add(created.workspace_id)
    const ws = { workspace_id: created.workspace_id, epoch: created.epoch }
    await setTheme(request, token, 'dark')
    // a workspace origin of its own, never opened in this browser: nothing stored there (the Zone host's login
    // page signs it in)
    await actor.page.goto(`${workspaceOrigin(actor, ws)}/w`)
    await expect(actor.page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', ws.workspace_id)
    await expect(actor.page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
    await expect(actor.page.locator('html')).toHaveAttribute('data-theme', 'dark')

    await setTheme(request, token, 'light')
    await actor.page.evaluate(() => window.dispatchEvent(new Event('focus')))
    await expect(actor.page.locator('html')).toHaveAttribute('data-theme', 'light')
    await setTheme(request, token, 'dark')
    await actor.page.evaluate(() => window.dispatchEvent(new Event('focus')))
    await expect(actor.page.locator('html')).toHaveAttribute('data-theme', 'dark')

    // opened again (a bookmark): the account's, not what this origin showed last
    await setTheme(request, token, 'light')
    await actor.page.reload()
    await expect(actor.page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', ws.workspace_id)
    await expect(actor.page.locator('html')).toHaveAttribute('data-theme', 'light')
    // the Desktop's session must exist again for the fixture's cleanup on the Zone host
    await actor.page.goto(`${origin}/`)
  } finally {
    await setTheme(request, token, before ?? 'light').catch(() => {})
    await request.dispose()
  }
})

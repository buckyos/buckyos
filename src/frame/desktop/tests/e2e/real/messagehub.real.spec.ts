import { createHash } from 'node:crypto'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'

/**
 * MessageHub against a real zone (UI_DATAMODEL §9.4 subset from the UI):
 * observer isolation, real timeline with request-box records, session
 * registration → first message → refresh, lifecycle, personal title, and
 * the attachment / delivery projections. Requires the proxied dev server
 * described in playwright.real.config.ts.
 */
const enabled = process.env.MESSAGEHUB_REAL_E2E === '1'
const adminUser = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const adminPassword = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025'
const agentDid = process.env.BUCKYOS_TEST_AGENT_DID || 'did:web:jarvis.test.buckyos.io'
const selfDid = `did:bns:${adminUser}`

type JsonRecord = Record<string, unknown>

function hashPassword(username: string, password: string, nonce: number): string {
  const original = createHash('sha256').update(`${password}${username}.buckyos`, 'utf8').digest('base64')
  return createHash('sha256').update(`${original}${nonce}`, 'utf8').digest('base64')
}

async function rpc(request: APIRequestContext, baseURL: string, service: string, method: string, params: JsonRecord, token?: string): Promise<JsonRecord> {
  const seq = Date.now()
  const response = await request.post(`${baseURL}/kapi/${service}`, { data: { method, params, sys: token ? [seq, token] : [seq] } })
  if (!response.ok()) throw new Error(`${method} HTTP ${response.status()}`)
  const body = await response.json() as JsonRecord
  if (body.error) throw new Error(String(body.error))
  return (body.result ?? {}) as JsonRecord
}

/**
 * API login and session injection. The control panel only accepts SSO
 * redirects to gateway-exposed ports, so the dev origin cannot complete the
 * browser SSO flow; the refresh cookie plus stored user info is exactly what
 * the SSO callback would have left behind.
 */
async function login(page: Page, request: APIRequestContext, baseURL: string): Promise<string> {
  const nonce = Date.now()
  const result = await rpc(request, baseURL, 'control-panel', 'auth.login', {
    username: adminUser,
    password: hashPassword(adminUser, adminPassword, nonce),
    appid: 'control-panel',
    target: { kind: 'system', service_id: 'control-panel' },
    login_nonce: nonce,
    remember_me: true,
  })
  const refresh = String(result.refresh_token)
  const url = new URL(baseURL)
  await page.context().addCookies([{ name: 'buckyos_refresh_token', value: refresh, domain: url.hostname, path: '/', httpOnly: true, secure: false, sameSite: 'Lax' }])
  await page.addInitScript((userInfo) => { window.localStorage.setItem('user_info', JSON.stringify(userInfo)) }, result.user_info)
  return String(result.session_token)
}

test.describe('MessageHub on a real zone', () => {
  test.skip(!enabled, 'Set MESSAGEHUB_REAL_E2E=1 with a proxied dev server (see playwright.real.config.ts).')

  test('observe the zone agent read-only and browse its real timeline', async ({ page, request, baseURL }) => {
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    await login(page, request, baseURL!)
    await page.goto(`/messagehub?${new URLSearchParams({ ownerDid: agentDid, mode: 'observe' })}`)
    await expect(page.getByTestId('owner-banner')).toBeVisible()
    await expect(page.getByTestId('conversation-history')).toBeVisible()
    await expect.poll(async () => Number(await page.getByTestId('conversation-history').getAttribute('data-raw-count'))).toBeGreaterThan(0)
    await expect(page.getByTestId('composer-readonly')).toBeVisible()
    await expect(page.locator('textarea')).toHaveCount(0)
    // Request-box records are surfaced per record; pick an entity that has any.
    await page.getByRole('button', { name: 'Requests', exact: true }).click()
    const requested = page.getByRole('main').locator('button').filter({ hasText: /\S/ }).filter({ has: page.locator('time') }).first()
    if (await requested.count()) {
      await requested.click()
      await expect(page.getByTestId('request-banner')).toBeVisible()
      await expect(page.getByTestId('request-chip').first()).toBeVisible()
      await expect(page.getByTestId('request-banner').getByRole('button')).toHaveCount(0)
    }
    await page.getByRole('button', { name: 'All', exact: true }).click()
    await page.getByRole('button', { name: 'Session details', exact: true }).last().click()
    const details = page.getByTestId('session-details')
    await expect(details).toBeVisible()
    await expect(details.getByRole('button', { name: 'Manage session', exact: true })).toBeDisabled()
    await expect(details.getByLabel('Personal display title')).toBeDisabled()
    await page.screenshot({ path: 'test-results-real/real-observer-1440.png' })
    expect(errors).toEqual([])
  })

  test('register a session with the agent, send, refresh, archive, restore and delete', async ({ page, request, baseURL }) => {
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    const token = await login(page, request, baseURL!)
    await page.goto('/messagehub')
    await expect(page.getByRole('heading', { name: 'MessageHub' })).toBeVisible()
    await page.getByRole('button', { name: 'New Session', exact: true }).first().click()
    const dialog = page.getByRole('dialog')
    await dialog.getByLabel('Entity').selectOption(agentDid)
    await dialog.getByLabel('Title (optional)').fill('UI 验收')
    await dialog.getByRole('button', { name: 'Create', exact: true }).click()
    await expect(dialog).toHaveCount(0)
    await expect(page.getByText('Start a conversation', { exact: true })).toBeVisible()
    const text = `MessageHub UI 验收 ${Date.now()}`
    await page.locator('textarea').fill(text)
    await page.locator('textarea').press('Enter')
    await expect(page.getByTestId('conversation-history').getByText(text, { exact: true })).toBeVisible()
    await expect(page.locator('textarea')).toHaveValue('')
    const sessionId = await page.evaluate(() => {
      const store = (window as unknown as { __messageHubStore: { defaultContext(): unknown; sessions(context: unknown): Array<{ id: string; shared: { title: string } }> } }).__messageHubStore
      return store.sessions(store.defaultContext()).find(session => session.shared.title === 'UI 验收')?.id ?? null
    })
    expect(sessionId).toBeTruthy()
    await page.reload()
    await expect(page.getByTestId('conversation-history').getByText(text, { exact: true })).toBeVisible()
    const state = await rpc(request, baseURL!, 'msg-center', 'msg.get_session_state', { owner: selfDid, session_id: sessionId }, token)
    expect(state.registered).toBe(true)
    const timeline = await rpc(request, baseURL!, 'msg-center', 'msg.list_session', { owner: selfDid, session_id: sessionId, with_object: true }, token)
    expect((timeline.items as JsonRecord[]).length).toBe(1)
    await page.screenshot({ path: 'test-results-real/real-session-1440.png' })

    const details = page.getByTestId('session-details')
    await page.getByRole('button', { name: 'Session details', exact: true }).last().click()
    await expect(details).toBeVisible()
    await details.getByLabel('Personal display title').fill('我的验收')
    await details.locator('form').filter({ has: page.getByLabel('Personal display title') }).getByRole('button', { name: 'Save', exact: true }).click()
    await expect(details.getByRole('heading', { name: '我的验收', exact: true })).toBeVisible()
    const stored = await rpc(request, baseURL!, 'msg-center', 'ui_session.get_state', { owner: selfDid, session_id: sessionId, key: 'ui.title' }, token)
    expect(stored.value).toBe('我的验收')
    await expect(details.getByLabel('Shared title', { exact: true })).toBeDisabled()

    await details.getByRole('button', { name: 'Manage session', exact: true }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Archive', exact: true }).click()
    await expect(page.getByRole('dialog')).toHaveCount(0)
    let listed = await rpc(request, baseURL!, 'msg-center', 'msg.list_sessions', { owner: selfDid, order_by: 'activity', lifecycle: 'archived' }, token)
    expect((listed.items as JsonRecord[]).some(item => item.session_id === sessionId)).toBe(true)
    await page.getByRole('button', { name: 'Sessions', exact: true }).click()
    await page.getByTestId('session-sidebar').getByRole('button', { name: /Archived/ }).click()
    const row = page.locator(`[data-session-id="${sessionId}"]`)
    await expect(row).toBeVisible()
    await row.getByRole('button', { name: /Manage session/ }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Restore', exact: true }).click()
    await expect(page.getByRole('dialog')).toHaveCount(0)
    listed = await rpc(request, baseURL!, 'msg-center', 'msg.list_sessions', { owner: selfDid, order_by: 'activity' }, token)
    expect((listed.items as JsonRecord[]).some(item => item.session_id === sessionId)).toBe(true)
    // Restoring the selected session returns the sidebar to the active list.
    await expect(page.getByTestId('session-sidebar').getByRole('button', { name: /Archived/ })).toBeVisible()
    await expect(row).toBeVisible()
    await row.getByRole('button', { name: /Manage session/ }).click()
    await page.getByRole('dialog').getByRole('button', { name: 'Delete permanently', exact: true }).click()
    await expect(page.getByRole('dialog')).toHaveCount(0)
    await expect(row).toHaveCount(0)
    const after = await rpc(request, baseURL!, 'msg-center', 'msg.list_session', { owner: selfDid, session_id: sessionId, with_object: true }, token)
    expect(((after.items as JsonRecord[]) ?? []).length).toBe(0)
    await page.reload()
    await expect(page.getByRole('heading', { name: 'MessageHub' })).toBeVisible()
    expect(await page.evaluate((id) => {
      const store = (window as unknown as { __messageHubStore: { defaultContext(): unknown; sessions(context: unknown): Array<{ id: string }> } }).__messageHubStore
      return store.sessions(store.defaultContext()).some(session => session.id === id)
    }, sessionId)).toBe(false)
    expect(errors).toEqual([])
  })
})

test.describe('MessageHub on a real zone (mobile and attachments)', () => {
  test.skip(!enabled, 'Set MESSAGEHUB_REAL_E2E=1 with a proxied dev server (see playwright.real.config.ts).')

  test('375px observer layout reaches the timeline, details and back path', async ({ page, request, baseURL }) => {
    await page.setViewportSize({ width: 375, height: 812 })
    await login(page, request, baseURL!)
    await page.goto(`/messagehub?${new URLSearchParams({ ownerDid: agentDid, mode: 'observe' })}`)
    await expect(page.getByTestId('owner-banner')).toBeVisible()
    await expect(page.getByTestId('conversation-history')).toBeVisible()
    await expect.poll(async () => Number(await page.getByTestId('conversation-history').getAttribute('data-raw-count'))).toBeGreaterThan(0)
    await page.screenshot({ path: 'test-results-real/real-observer-375.png' })
    await page.getByRole('button', { name: 'Session details', exact: true }).last().click()
    await expect(page.getByTestId('session-details')).toBeVisible()
    await page.getByTestId('session-details').getByRole('button', { name: 'Close', exact: true }).click()
    await expect(page.getByTestId('conversation-history')).toBeVisible()
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  })

  test('send a file attachment: NDM upload, object reference and authenticated download', async ({ page, request, baseURL }) => {
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    const token = await login(page, request, baseURL!)
    await page.goto('/messagehub')
    await page.getByRole('button', { name: 'New Session', exact: true }).first().click()
    const dialog = page.getByRole('dialog')
    await dialog.getByLabel('Entity').selectOption(agentDid)
    await dialog.getByLabel('Title (optional)').fill('附件验收')
    await dialog.getByRole('button', { name: 'Create', exact: true }).click()
    await expect(dialog).toHaveCount(0)
    const sessionId = await page.evaluate(() => {
      const store = (window as unknown as { __messageHubStore: { defaultContext(): unknown; sessions(context: unknown): Array<{ id: string; shared: { title: string } }> } }).__messageHubStore
      return store.sessions(store.defaultContext()).find(session => session.shared.title === '附件验收')?.id ?? null
    })
    expect(sessionId).toBeTruthy()
    try {
      await page.locator('input[type="file"]').first().setInputFiles('package.json')
      await page.locator('textarea').fill('attachment check')
      await page.locator('textarea').press('Enter')
      const outcome = page.getByTestId('attachment-ready').first().or(page.getByRole('alert'))
      await expect(outcome).toBeVisible({ timeout: 60_000 })
      const alert = page.getByRole('alert')
      if (await alert.count()) {
        test.info().annotations.push({ type: 'attachment-upload', description: await alert.first().innerText() })
        throw new Error(`attachment send failed: ${await alert.first().innerText()}`)
      }
      await expect(page.getByTestId('attachment-ready').first()).toContainText('package.json')
      const timeline = await rpc(request, baseURL!, 'msg-center', 'msg.list_session', { owner: selfDid, session_id: sessionId, with_object: true }, token)
      const item = (timeline.items as JsonRecord[])[0]
      const refs = ((item.msg as JsonRecord).content as JsonRecord).refs as JsonRecord[]
      expect(refs.length).toBe(1)
      const objId = String((refs[0].target as JsonRecord).obj_id)
      expect(objId.startsWith('cyfile:')).toBe(true)
      const unauthenticated = await request.get(`${baseURL}/kapi/msg-center/objects/${encodeURIComponent(objId)}/content`)
      expect(unauthenticated.status()).toBe(401)
      const content = await request.get(`${baseURL}/kapi/msg-center/objects/${encodeURIComponent(objId)}/content`, { headers: { authorization: `Bearer ${token}` } })
      expect(content.status()).toBe(200)
      expect((await content.text()).includes('"name": "buckyos-web-desktop"')).toBe(true)
      await page.screenshot({ path: 'test-results-real/real-attachment-1440.png' })
      expect(errors).toEqual([])
    } finally {
      await rpc(request, baseURL!, 'msg-center', 'msg.delete_session', { owner: selfDid, session_id: sessionId }, token).catch(() => undefined)
    }
  })
})

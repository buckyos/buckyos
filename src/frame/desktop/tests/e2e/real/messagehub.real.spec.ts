import { createHash, randomBytes } from 'node:crypto'
import { crc32, deflateSync } from 'node:zlib'
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

  test('a session link from the agent home page opens that session', async ({ page, request, baseURL }) => {
    await login(page, request, baseURL!)
    const active = async () => {
      const toggle = page.getByRole('button', { name: 'Sessions', exact: true })
      await expect(toggle).toBeVisible()
      if (!(await page.getByTestId('session-sidebar').isVisible())) await toggle.click()
      return page.locator('[data-testid="session-row"]:has(button[aria-current="true"])')
    }
    // The owner's own conversation with the agent.
    await page.goto(`/messagehub?${new URLSearchParams({ entityId: agentDid, sessionId: `dm:${agentDid}` })}`)
    await expect(await active()).toHaveAttribute('data-session-id', `dm:${agentDid}`)
  })

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
    await page.getByTestId('hub-create-menu').click()
    await page.getByRole('menuitem', { name: 'New Session', exact: true }).click()
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
    await page.getByTestId('hub-create-menu').click()
    await page.getByRole('menuitem', { name: 'New Session', exact: true }).click()
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
      const alert = page.getByRole('alert')
      await expect.poll(async () => (await alert.count()) > 0 || ((await page.getByTestId('upload-progress').count()) === 0 && (await page.getByTestId('attachment-ready').count()) > 0), { timeout: 60_000 }).toBe(true)
      if (await alert.count()) {
        test.info().annotations.push({ type: 'attachment-upload', description: await alert.first().innerText() })
        throw new Error(`attachment send failed: ${await alert.first().innerText()}`)
      }
      await expect(page.getByTestId('attachment-ready').first()).toContainText('package.json')
      await expect(page.getByTestId('conversation-history').getByTestId('delivery-status').first()).not.toHaveAttribute('data-status', 'sending', { timeout: 30_000 })
      const timeline = await rpc(request, baseURL!, 'msg-center', 'msg.list_session', { owner: selfDid, session_id: sessionId, with_object: true }, token)
      const item = (timeline.items as JsonRecord[])[0]
      const refs = ((item.msg as JsonRecord).content as JsonRecord).refs as JsonRecord[]
      expect(refs.length).toBe(1)
      const objId = String((refs[0].target as JsonRecord).obj_id)
      expect(objId.startsWith('cyfile:')).toBe(true)
      const contentUrl = `${baseURL}/kapi/msg-center/objects/${encodeURIComponent(objId)}/content?context_path=${encodeURIComponent(String(item.msg_id))}`
      const unauthenticated = await request.get(contentUrl)
      expect(unauthenticated.status()).toBe(401)
      const content = await request.get(contentUrl, { headers: { authorization: `Bearer ${token}` } })
      expect(content.status()).toBe(200)
      expect((await content.text()).includes('"name": "buckyos-web-desktop"')).toBe(true)
      await page.screenshot({ path: 'test-results-real/real-attachment-1440.png' })
      expect(errors).toEqual([])
    } finally {
      await rpc(request, baseURL!, 'msg-center', 'msg.delete_session', { owner: selfDid, session_id: sessionId }, token).catch(() => undefined)
    }
  })

  test('on a slow uplink a picture shows at once with its upload progress, then becomes the stored message', async ({ page, request, baseURL }) => {
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    const token = await login(page, request, baseURL!)
    await page.setViewportSize({ width: 390, height: 844 })
    await page.goto('/messagehub')
    await page.getByTestId('hub-create-menu').click()
    await page.getByRole('menuitem', { name: 'New Session', exact: true }).click()
    const dialog = page.getByRole('dialog')
    await dialog.getByLabel('Entity').selectOption(agentDid)
    await dialog.getByLabel('Title (optional)').fill('图片占位验收')
    await dialog.getByRole('button', { name: 'Create', exact: true }).click()
    await expect(dialog).toHaveCount(0)
    const sessionId = await page.evaluate(() => {
      const store = (window as unknown as { __messageHubStore: { defaultContext(): unknown; sessions(context: unknown): Array<{ id: string; shared: { title: string } }> } }).__messageHubStore
      return store.sessions(store.defaultContext()).find(session => session.shared.title === '图片占位验收')?.id ?? null
    })
    expect(sessionId).toBeTruthy()
    const cdp = await page.context().newCDPSession(page)
    try {
      await cdp.send('Network.enable')
      await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: 150, downloadThroughput: 256 * 1024, uploadThroughput: 64 * 1024 })
      const history = page.getByTestId('conversation-history')
      await page.locator('input[type="file"]').first().setInputFiles({ name: 'slow-uplink.png', mimeType: 'image/png', buffer: noisePng(320, 240) })
      await page.locator('textarea').fill('slow uplink picture')
      const started = Date.now()
      await page.locator('textarea').press('Enter')
      const progress = history.getByTestId('upload-progress')
      await expect(progress).toBeVisible({ timeout: 2_000 })
      await expect(history.getByRole('img', { name: 'slow-uplink.png', exact: true })).toBeVisible({ timeout: 2_000 })
      await expect(progress).toHaveAttribute('data-phase', 'uploading', { timeout: 30_000 })
      await expect.poll(async () => Number(await progress.getAttribute('data-percent') ?? 0), { timeout: 30_000 }).toBeGreaterThan(0)
      await page.screenshot({ path: 'test-results-real/real-upload-progress-390.png' })
      await expect(progress).toHaveCount(0, { timeout: 90_000 })
      test.info().annotations.push({ type: 'upload-ms', description: String(Date.now() - started) })
      await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1 })
      await expect(history.getByTestId('delivery-status').first()).not.toHaveAttribute('data-status', 'sending', { timeout: 30_000 })
      await expect(history.getByRole('img', { name: 'slow-uplink.png', exact: true })).toBeVisible()
      const timeline = await rpc(request, baseURL!, 'msg-center', 'msg.list_session', { owner: selfDid, session_id: sessionId, with_object: true }, token)
      const msg = (timeline.items as JsonRecord[])[0].msg as JsonRecord
      const refs = (msg.content as JsonRecord).refs as JsonRecord[]
      expect(refs.map(ref => ref.label)).toEqual(['slow-uplink.png'])
      expect(String((refs[0].target as JsonRecord).obj_id).startsWith('cyfile:')).toBe(true)
      expect(Number(msg.created_at_ms)).toBeLessThanOrEqual(started + 1_000)
      await page.screenshot({ path: 'test-results-real/real-upload-done-390.png' })
      expect(errors).toEqual([])
    } finally {
      await cdp.detach().catch(() => undefined)
      await rpc(request, baseURL!, 'msg-center', 'msg.delete_session', { owner: selfDid, session_id: sessionId }, token).catch(() => undefined)
    }
  })
})

function noisePng(width: number, height: number): Buffer {
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
    const length = Buffer.alloc(4)
    length.writeUInt32BE(data.length)
    const crc = Buffer.alloc(4)
    crc.writeUInt32BE(crc32(body))
    return Buffer.concat([length, body, crc])
  }
  const header = Buffer.alloc(13)
  header.writeUInt32BE(width, 0)
  header.writeUInt32BE(height, 4)
  header[8] = 8
  header[9] = 2
  const rows = Array.from({ length: height }, () => Buffer.concat([Buffer.from([0]), randomBytes(width * 3)]))
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', header), chunk('IDAT', deflateSync(Buffer.concat(rows))), chunk('IEND', Buffer.alloc(0))])
}

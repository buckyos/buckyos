// Self-host Group extension layers in a real browser against a running Zone
// (notepads/self-host-group-extension-todo.md EXT-08): L1 declarations, the L2 panel host
// (trusted App-subdomain panel loads, a dead App host shows "unavailable" while the chat keeps
// working), display rendering, and the guest page beside its panels. Run:
//   BUCKYOS_TEST_ZONE_HOST=test.buckyos.io BUCKYOS_TEST_APP_HOST=buckyos-systest \
//   pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/group-extensions.spec.ts
import { createHash } from 'node:crypto'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'

type JsonRecord = Record<string, unknown>

const zoneHost = process.env.BUCKYOS_TEST_ZONE_HOST || 'test.buckyos.io'
const zoneOrigin = `https://${zoneHost}`
const adminUser = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const adminPassword = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025'
/** An App actually served by the gateway on `<label>.<zone>`; its root page is the "panel". */
const appHost = process.env.BUCKYOS_TEST_APP_HOST || 'buckyos-systest'
const runId = Date.now().toString(36)

function hashPassword(username: string, password: string, nonce?: number): string {
  const original = createHash('sha256').update(`${password}${username}.buckyos`, 'utf8').digest('base64')
  if (nonce === undefined) return original
  return createHash('sha256').update(`${original}${nonce}`, 'utf8').digest('base64')
}

async function rpc(request: APIRequestContext, service: string, method: string, params: JsonRecord, token?: string): Promise<JsonRecord> {
  const seq = Date.now()
  const response = await request.post(`${zoneOrigin}/kapi/${service}`, {
    data: { method, params, sys: token ? [seq, token] : [seq] },
    ignoreHTTPSErrors: true,
  })
  if (!response.ok()) throw new Error(`${method} HTTP ${response.status()}`)
  const body = await response.json() as JsonRecord
  if (body.error) throw new Error(`${method}: ${String(body.error)} ${JSON.stringify(body.data ?? null)}`)
  return (body.result ?? {}) as JsonRecord
}

async function adminToken(request: APIRequestContext): Promise<string> {
  const nonce = Date.now()
  const result = await rpc(request, 'control-panel', 'auth.login', {
    username: adminUser,
    password: hashPassword(adminUser, adminPassword, nonce),
    target: { kind: 'system', service_id: 'control-panel' },
    login_nonce: nonce,
  })
  return String(result.session_token)
}

/** The Desktop keeps the UI language per browser; the test asserts English labels. */
async function fixLocale(page: Page) {
  await page.addInitScript(() => {
    try { window.localStorage.setItem('buckyos.prototype.locale.v1', 'en') } catch { /* no storage */ }
  })
}

async function signIn(page: Page) {
  await fixLocale(page)
  await page.goto(`${zoneOrigin}/`)
  await page.getByLabel('Username').fill(adminUser)
  await page.getByLabel('Password').fill(adminPassword)
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  await expect(page.getByTestId('desktop-taskbar')).toBeVisible()
}

test('panels from the group host load in Message Hub, a dead panel is unavailable, display renders, the guest sees the same', async ({ page, browser, request }) => {
  const token = await adminToken(request)
  const identity = await rpc(request, 'control-panel', 'auth.current_identity', {}, token)
  const name = `扩展验收群 ${runId}`
  const created = await rpc(request, 'msg-center', 'group.create', {
    idempotency_key: `group-ext-ui-${runId}`,
    profile: { name },
    configuration: {
      required_features: ['panels'],
      display: {
        announcement: `公告 ${runId}`,
        links: [{ title: '帮助中心', url: `https://${appHost}.${zoneHost}/help` }],
        featured_members: [identity.did],
      },
      extensions: {
        panels: [
          { id: 'live', title: '工单', entry_url: `https://${appHost}.${zoneHost}/?from=panel`, scope: 'group' },
          { id: 'dead', title: '离线面板', entry_url: `https://nosuchapp-${runId}.${zoneHost}/`, scope: { sessions: ['tickets'] } },
        ],
      },
      session_templates: { support: { membership: { roles: ['owner', 'admin'] }, rules: { allow_guests: true, display: { announcement: `工单公告 ${runId}` } } } },
    },
    sessions: [{ session_id: 'tickets', template: 'support', title: `工单会话 ${runId}` }],
  }, token)
  const groupDid = String(created.group_did)
  const sessions = await rpc(request, 'msg-center', 'group.list_sessions', { group_did: groupDid }, token)
  const tickets = (sessions.items as JsonRecord[]).find((s) => s.session_id === 'tickets')!
  expect((tickets.panels as JsonRecord[]).map((p) => p.id)).toEqual(['live', 'dead'])
  await expect(async () => {
    expect(await rpc(request, 'msg-center', 'group.apply_config', {
      group_did: groupDid, expected_revision: created.revision, idempotency_key: `evil-${runId}`,
      patch: { extensions: { panels: [{ id: 'evil', title: 'x', entry_url: 'https://evil.example/p' }] } },
    }, token)).toBeUndefined()
  }).rejects.toThrow(/untrusted-origin/)

  try {
    // A panel whose App is unreachable at the network level (connection dropped). The gateway
    // answers an *unknown* App host with an empty 403 page, which a cross-origin frame cannot
    // tell from content, so the dead case is produced by dropping the connection.
    await page.route(`https://nosuchapp-${runId}.${zoneHost}/**`, (route) => route.abort('connectionrefused'))
    await signIn(page)
    await page.goto(`${zoneOrigin}/messagehub?entityId=${encodeURIComponent(groupDid)}`)
    await page.getByRole('button', { name: new RegExp(`^(Entity details|实体详情): ${name}`) }).last().click()
    const extensions = page.getByTestId('group-panel').getByTestId('group-extensions')
    await expect(extensions.getByTestId('display-announcement')).toContainText(`公告 ${runId}`)
    await expect(extensions.getByTestId('display-links').getByRole('link', { name: '帮助中心' })).toHaveAttribute('href', `https://${appHost}.${zoneHost}/help`)
    const panels = extensions.getByTestId('group-panels')
    await expect(panels.getByTestId('group-panels-toggle')).toContainText('1')
    await panels.getByTestId('group-panels-toggle').click()
    const frame = panels.getByTestId('group-panel-frame')
    await expect(frame).toHaveAttribute('data-state', 'ready', { timeout: 20_000 })
    const src = new URL((await frame.locator('iframe').getAttribute('src'))!)
    expect(src.host).toBe(`${appHost}.${zoneHost}`)
    expect(src.searchParams.get('group_did')).toBe(groupDid)
    expect(src.searchParams.get('session')).toBe(groupDid)
    expect(src.searchParams.get('from')).toBe('panel')
    expect(src.searchParams.has('token')).toBe(false)
    await page.screenshot({ path: 'test-results/dv-group-ext-panel.png', fullPage: true })

    // The named session: the live panel plus the dead one, which ends up unavailable while chat still works.
    await page.getByRole('button', { name: /^(Close|关闭)$/ }).last().click()
    await page.getByRole('button', { name: /^(Sessions|会话)$/ }).click()
    await page.locator('[data-testid="session-row"]', { hasText: `工单会话 ${runId}` }).getByRole('button', { name: new RegExp(`工单会话 ${runId}`) }).first().click()
    await page.getByRole('button', { name: /^(Session details|会话详情)$/ }).last().click()
    const details = page.getByTestId('session-details')
    await expect(details.getByTestId('session-display').getByTestId('display-announcement')).toContainText(`工单公告 ${runId}`)
    const sessionPanels = details.getByTestId('group-panels')
    await sessionPanels.getByTestId('group-panels-toggle').click()
    await expect(sessionPanels.getByRole('tab')).toHaveText(['工单', '离线面板'])
    await sessionPanels.getByRole('tab', { name: '离线面板' }).click()
    await expect(sessionPanels.getByTestId('group-panel-unavailable')).toBeVisible({ timeout: 30_000 })
    const composer = page.getByPlaceholder(/^(Message|输入消息)/).first()
    await composer.fill(`面板离线时照常发言 ${runId}`)
    await composer.press('Enter')
    await expect(page.getByText(`面板离线时照常发言 ${runId}`).first()).toBeVisible()
    await page.screenshot({ path: 'test-results/dv-group-ext-session-dead-panel.png', fullPage: true })

    // An external customer through an exclusive invite sees the panels and display beside the conversation.
    const invite = await rpc(request, 'control-panel', 'user.exclusive_invite.create', {
      new_user: { user_id: `extcust${runId}`, show_name: '扩展客户' },
      channel: { kind: 'email', target_hint: 'e***@example.com' },
      resource: { kind: 'group_session', group_did: groupDid, session_id: 'tickets', title: '工单' },
    }, token)
    const context = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1280, height: 900 } })
    const guest = await context.newPage()
    await fixLocale(guest)
    await guest.goto(String(invite.link))
    await guest.waitForURL(/\/messagehub\/guest\?/)
    await guest.getByTestId('guest-accept').click()
    await expect(guest.getByTestId('guest-conversation')).toBeVisible()
    const side = guest.getByTestId('guest-side-panel')
    await expect(side.getByTestId('display-announcement')).toContainText(`工单公告 ${runId}`)
    // The guest page opens its panels by default.
    await expect(side.getByRole('tab')).toHaveText(['工单', '离线面板'])
    await expect(side.getByTestId('group-panel-frame')).toHaveAttribute('data-state', 'ready', { timeout: 20_000 })
    await guest.screenshot({ path: 'test-results/dv-group-ext-guest.png', fullPage: true })
    await context.close()
  } finally {
    await rpc(request, 'msg-center', 'group.delete', { group_did: groupDid }, token).catch(() => {})
  }
})

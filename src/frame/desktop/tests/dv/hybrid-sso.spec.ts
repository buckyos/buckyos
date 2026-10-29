// Hybrid SSO in a real browser against a running Zone (notepads/hybrid-sso-implementation-todo.md
// SSO-14): real cookies, the invite link with its fragment secret, the Message Hub guest page and the
// share entry page. Run: BUCKYOS_TEST_ZONE_HOST=test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=bucky2025 \
//   pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/hybrid-sso.spec.ts
import { createHash } from 'node:crypto'
import { expect, test, type APIRequestContext } from '@playwright/test'

type JsonRecord = Record<string, unknown>

const zoneHost = process.env.BUCKYOS_TEST_ZONE_HOST || 'test.buckyos.io'
const zoneOrigin = `https://${zoneHost}`
const adminUser = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const adminPassword = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025'
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
  const result = (body.result ?? {}) as JsonRecord
  if (result.ok === false && result.error) throw new Error(`${method}: ${JSON.stringify(result.error)}`)
  return result
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

test('an exclusive invite opens the guest Session: join, post, the host sees it', async ({ page, request }) => {
  const token = await adminToken(request)
  const created = await rpc(request, 'msg-center', 'group.create', {
    idempotency_key: `hsso-ui-${runId}`,
    configuration: {
      profile: { name: `混合 SSO 浏览器验收 ${runId}` },
      session_templates: { tickets: { membership: 'explicit', rules: { allow_guests: true } } },
    },
  }, token)
  const groupDid = String(created.group_did)
  await rpc(request, 'msg-center', 'group.create_session', {
    group_did: groupDid,
    session_id: 'ticket-ui',
    membership: 'explicit',
    rule_overrides: { allow_guests: true },
  }, token)
  const invite = await rpc(request, 'control-panel', 'user.exclusive_invite.create', {
    new_user: { user_id: `uicust${runId}`, show_name: '浏览器客户' },
    channel: { kind: 'email', target_hint: 'u***@example.com' },
    resource: { kind: 'group_session', group_did: groupDid, session_id: 'ticket-ui', title: '浏览器工单' },
  }, token)
  const link = String(invite.link)
  expect(new URL(link).hash.startsWith('#s=')).toBe(true)

  await page.goto(link)
  await page.waitForURL(/\/messagehub\/guest\?/)
  // The secret never stays in the address bar or reaches the server in a URL.
  expect(page.url()).not.toContain('#s=')
  await page.getByTestId('guest-accept').click()
  await expect(page.getByTestId('guest-conversation')).toBeVisible()
  const composer = page.getByRole('textbox').last()
  await composer.fill(`浏览器留言 ${runId}`)
  await composer.press('Enter')
  await expect(page.getByText(`浏览器留言 ${runId}`)).toBeVisible()

  const listed = await rpc(request, 'msg-center', 'group.list_messages', { group_did: groupDid, session_id: 'ticket-ui', after_seq: 0, limit: 20, with_object: true }, token)
  expect(JSON.stringify(listed)).toContain(`浏览器留言 ${runId}`)

  // The restricted identity reaches only what its session opens.
  const cookies = await page.context().cookies(zoneOrigin)
  const session = cookies.find((cookie) => cookie.name === 'buckyos_session_token' && cookie.domain === zoneHost)
  expect(session).toBeTruthy()
  const identity = await rpc(request, 'control-panel', 'auth.current_identity', {}, session!.value)
  expect(identity).toMatchObject({ kind: 'account', restricted: true, auth_method: 'exclusive_invite', password_set: false })
})

test('a guest entry of an AI Workspace opens the workspace without an account', async ({ browser, request }) => {
  const token = await adminToken(request)
  const ws = await rpc(request, 'aiworkspace', 'ws.create', { title: `DJ 电台 ${runId}` }, token)
  const workspaceId = String(ws.workspace_id)
  const entry = await rpc(request, 'aiworkspace', 'ws.create_entry', {
    workspace_id: workspaceId,
    capabilities: ['read', 'comment'],
    methods: { guest: true, password: false, self_register: true, did_login: false },
  }, token)
  const entryId = String((entry.entry as JsonRecord).entry_id)

  const context = await browser.newContext({ ignoreHTTPSErrors: true })
  const page = await context.newPage()
  await page.goto(`${zoneOrigin}/sso/entry?service=aiworkspace&entry=${encodeURIComponent(entryId)}`)
  await expect(page.getByTestId('sso-entry-method-guest')).toBeVisible()
  await expect(page.getByTestId('sso-entry-method-self_register')).toBeVisible()
  await expect(page.getByTestId('sso-entry-method-did_login')).toHaveCount(0)
  await page.getByTestId('sso-entry-method-guest').click()
  await page.waitForURL(new RegExp(`^https://aiws-${workspaceId.replace(/^ws_/, '')}\\.`))
  await expect(page.getByTestId('aiws-open-error')).toHaveCount(0)
  await context.close()

  await rpc(request, 'aiworkspace', 'ws.close_entry', { workspace_id: workspaceId, entry_id: entryId }, token)
  const closed = await rpc(request, 'control-panel', 'auth.entry.describe', { service: 'aiworkspace', entry_id: entryId })
  expect(closed.state).toBe('unavailable')
})

test('a Limited account signed in with its password keeps the Desktop', async ({ browser, request }) => {
  const token = await adminToken(request)
  const nonce = Date.now() + 3
  const sudo = await rpc(request, 'verify-hub', 'sudo_by_password', {
    username: adminUser,
    password: hashPassword(adminUser, adminPassword, nonce),
    target: { kind: 'system', service_id: 'control-panel' },
    aud: 'system-config',
    login_nonce: nonce,
  })
  const userId = `uilim${runId}`
  await rpc(request, 'control-panel', 'user.create', {
    user_id: userId,
    password_hash: hashPassword(userId, 'limited-pass-1'),
    user_type: 'limited',
  }, String(sudo.session_token))
  expect(token).toBeTruthy()

  const context = await browser.newContext({ ignoreHTTPSErrors: true })
  const page = await context.newPage()
  await page.goto(`${zoneOrigin}/`)
  await page.getByLabel('Username').fill(userId)
  await page.getByLabel('Password').fill('limited-pass-1')
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  await expect(page.getByTestId('desktop-taskbar')).toBeVisible()
  await expect(page.getByTestId('restricted-home')).toHaveCount(0)
  await context.close()
})

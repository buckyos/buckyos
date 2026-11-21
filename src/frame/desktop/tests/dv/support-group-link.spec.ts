// sys-test official example in a real browser against a running Zone
// (notepads/done/self-host-group-extension-todo.md §11, EXT-10): the installing account clicks
// the two buttons on the sys_test page, a customer opens the 48-hour link (set-password
// prompt, join, chat beside the App panel), sets a password, and the same link then asks
// for the password. Run:
//   BUCKYOS_TEST_ZONE_HOST=test.buckyos.io BUCKYOS_TEST_APP_HOST=buckyos-systest \
//   pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/support-group-link.spec.ts
import { createHash } from 'node:crypto'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'

type JsonRecord = Record<string, unknown>

const zoneHost = process.env.BUCKYOS_TEST_ZONE_HOST || 'test.buckyos.io'
const zoneOrigin = `https://${zoneHost}`
const adminUser = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const adminPassword = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025'
const appHost = process.env.BUCKYOS_TEST_APP_HOST || 'buckyos-systest'
const appId = process.env.BUCKYOS_TEST_APP_ID || 'buckyos-systest.buckyos.bns.did'
const appOrigin = `https://${appHost}.${zoneHost}`
const runId = Date.now().toString(36)
const members = (process.env.BUCKYOS_TEST_MEMBERS || 'alice,bob').split(',').map(item => item.trim())
const hostRules = process.env.BUCKYOS_TEST_HOST_RULES
const proxy = process.env.BUCKYOS_TEST_PROXY
test.use({
  trace: 'off',
  screenshot: 'off',
  video: 'off',
  ...(proxy ? { proxy: { server: proxy } } : {}),
  ...(hostRules ? { launchOptions: { args: ['--ignore-certificate-errors', `--host-resolver-rules=${hostRules}`] } } : {}),
})

function hashPassword(username: string, password: string, nonce?: number): string {
  const original = createHash('sha256').update(`${password}${username}.buckyos`, 'utf8').digest('base64')
  if (nonce === undefined) return original
  return createHash('sha256').update(`${original}${nonce}`, 'utf8').digest('base64')
}

async function rpc(request: APIRequestContext, service: string, method: string, params: JsonRecord, token?: string): Promise<JsonRecord> {
  const seq = Date.now()
  const response = await request.post(`${zoneOrigin}/kapi/${service}`, { data: { method, params, sys: token ? [seq, token] : [seq] }, ignoreHTTPSErrors: true })
  if (!response.ok()) throw new Error(`${method} HTTP ${response.status()}`)
  const body = await response.json() as JsonRecord
  if (body.error) throw new Error(`${method}: ${String(body.error)} ${JSON.stringify(body.data ?? null)}`)
  return (body.result ?? {}) as JsonRecord
}

async function adminToken(request: APIRequestContext): Promise<string> {
  const nonce = Date.now()
  const result = await rpc(request, 'control-panel', 'auth.login', {
    username: adminUser, password: hashPassword(adminUser, adminPassword, nonce), target: { kind: 'system', service_id: 'control-panel' }, login_nonce: nonce,
  })
  return String(result.session_token)
}

async function fixLocale(page: Page) {
  await page.addInitScript(() => {
    try { window.localStorage.setItem('buckyos.prototype.locale.v1', 'en') } catch { /* no storage */ }
  })
}

/** The sys_test page is public on the App origin; its Login button starts the SDK SSO flow through the Zone login page. */
async function openSysTest(page: Page) {
  await fixLocale(page)
  await page.goto(`${appOrigin}/`)
  const login = page.locator('#btn-login')
  await expect(login).toBeEnabled({ timeout: 30_000 })
  if ((await page.locator('#auth-status').textContent())?.includes(adminUser) !== true) {
    await login.click()
    const username = page.getByLabel('Username')
    if (await username.waitFor({ state: 'visible', timeout: 15_000 }).then(() => true, () => false)) {
      await username.fill(adminUser)
      await page.getByLabel('Password').fill(adminPassword)
      await page.getByRole('button', { name: 'Sign In', exact: true }).click()
    }
  }
  await expect(page.locator('#auth-status')).toContainText(adminUser, { timeout: 30_000 })
}

test('owner creates the support group and a customer link; the customer joins, sets a password, then needs it', async ({ page, browser, request }) => {
  const token = await adminToken(request)
  const instance = { app_instance_id: `${appId}@${adminUser}` }
  const previous = await rpc(request, 'control-panel', 'apps.app_login.get', instance, token)
  const wasEnabled = (previous.settings as JsonRecord).enabled === true
  let groupDid = ''
  try {
    await rpc(request, 'control-panel', 'apps.app_login.set', { app_instance_id: `${appId}@${adminUser}`, enabled: true }, token)

    await openSysTest(page)
    const card = page.locator('[data-group="support_group"]')
    await card.locator('#sg-name').fill(`客服群（sys-test UI ${runId}）`)
    await card.locator('#sg-member-a').fill(members[0] || '')
    await card.locator('#sg-member-b').fill(members[1] || '')
    await card.getByTestId('support-create').click()
    const groupRow = card.getByTestId('support-group-did')
    await expect(groupRow).toBeVisible({ timeout: 30_000 })
    groupDid = (await groupRow.locator('code').first().textContent())!.trim()
    expect(groupDid).toMatch(/^did:/)
    await card.locator('#sg-customer-name').fill('UI 客户')
    await card.locator('#sg-customer-id').fill(`ui:${runId}`)
    await card.getByTestId('support-link').click()
    const linkNode = card.getByTestId('support-customer-link')
    await expect(linkNode).toBeVisible({ timeout: 30_000 })
    const link = (await linkNode.textContent())!.trim()
    const invitation = new URL(link)
    expect(invitation.origin).toBe(zoneOrigin)
    expect(invitation.pathname).toMatch(/^\/sso\/invite\/[^/]+$/)
    expect(new URLSearchParams(invitation.hash.slice(1)).has('s')).toBe(true)

    // The customer, no password yet: the link signs in, the guest page offers (not forces) a password.
    const customer = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1280, height: 900 } })
    const guest = await customer.newPage()
    await fixLocale(guest)
    await guest.goto(link)
    await guest.waitForURL(url => url.origin === appOrigin && url.pathname === '/support/ticket')
    const cookies = await customer.cookies(zoneOrigin)
    expect(cookies.some(c => c.name === 'buckyos_session_token')).toBe(false)
    const join = guest.locator('#ticket-join')
    const composer = guest.locator('#ticket-message textarea')
    await expect(join.or(composer).filter({ visible: true })).toBeVisible()
    if (await join.isVisible()) await join.click()
    await composer.fill(`客户留言 ${runId}`)
    await guest.locator('#ticket-message button').click()
    await expect(guest.locator('#ticket-messages')).toContainText(`客户留言 ${runId}`)
    const passForm = guest.locator('#ticket-password')
    await passForm.locator('input').nth(0).fill(`ui-pass-${runId}`)
    await passForm.locator('input').nth(1).fill(`ui-pass-${runId}`)
    await passForm.locator('button').click()
    await expect(guest.locator('#ticket-status')).toContainText('密码已设置')
    const userId = (await guest.locator('#ticket-status').textContent())!.split(' ').pop()
    expect(userId).toBeTruthy()
    await customer.close()

    // The same link on another device: password first, then the same conversation.
    const later = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1280, height: 900 } })
    const again = await later.newPage()
    await fixLocale(again)
    await again.goto(link)
    await expect(again.getByTestId('sso-invite-password-login')).toBeVisible()
    await again.getByTestId('sso-invite-password-login').click()
    await expect(again.getByLabel('Username')).toHaveValue(userId!)
    await again.getByLabel('Password').fill(`ui-pass-${runId}`)
    await again.getByRole('button', { name: 'Sign In', exact: true }).click()
    await again.waitForURL(/\/messagehub\/guest\?/)
    await expect(again.getByTestId('guest-conversation')).toBeVisible({ timeout: 30_000 })
    await expect(again.getByText(`客户留言 ${runId}`).first()).toBeVisible()
    await again.screenshot({ path: 'test-results/dv-support-group-password.png', fullPage: true })
    await later.close()

    // The owner, in Message Hub, sees the customer's message beside the App panel.
    const hubContext = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1280, height: 900 } })
    const hub = await hubContext.newPage()
    await fixLocale(hub)
    await hub.goto(`${zoneOrigin}/`)
    await hub.getByLabel('Username').fill(adminUser)
    await hub.getByLabel('Password').fill(adminPassword)
    await hub.getByRole('button', { name: 'Sign In', exact: true }).click()
    await expect(hub.getByTestId('desktop-taskbar')).toBeVisible()
    await hub.goto(`${zoneOrigin}/messagehub?entityId=${encodeURIComponent(groupDid)}`)
    await hub.getByRole('button', { name: /^(Sessions|会话)$/ }).click()
    await hub.locator('[data-testid="session-row"]', { hasText: '客服工单' }).first().click()
    await expect(hub.getByText(`客户留言 ${runId}`).first()).toBeVisible({ timeout: 30_000 })
    await hub.screenshot({ path: 'test-results/dv-support-group-hub.png', fullPage: true })
    await hubContext.close()
  } finally {
    if (groupDid) await rpc(request, 'msg-center', 'group.delete', { group_did: groupDid }, token)
    if (!wasEnabled) await rpc(request, 'control-panel', 'apps.app_login.set', { ...instance, enabled: false }, token)
  }
})

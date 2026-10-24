// sys-test official example in a real browser against a running Zone
// (notepads/self-host-group-extension-todo.md §11, EXT-10): the installing account clicks
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
  await rpc(request, 'control-panel', 'apps.app_login.set', { app_instance_id: `${appId}@${adminUser}`, enabled: true }, token)

  await openSysTest(page)
  const card = page.locator('[data-group="support_group"]')
  await card.locator('#sg-name').fill(`客服群（sys-test UI ${runId}）`)
  await card.getByTestId('support-create').click()
  const groupRow = card.getByTestId('support-group-did')
  await expect(groupRow).toBeVisible({ timeout: 30_000 })
  const groupDid = (await groupRow.locator('code').first().textContent())!.trim()
  expect(groupDid).toMatch(/^did:/)
  try {
    await card.locator('#sg-customer-name').fill('UI 客户')
    await card.locator('#sg-customer-id').fill(`ui:${runId}`)
    await card.getByTestId('support-link').click()
    const linkNode = card.getByTestId('support-customer-link')
    await expect(linkNode).toBeVisible({ timeout: 30_000 })
    const link = (await linkNode.textContent())!.trim()
    expect(link).toMatch(new RegExp(`^https://${zoneHost.replace(/\\./g, '\\\\.')}/sso/invite/[^#]+#s=`))
    await page.screenshot({ path: 'test-results/dv-support-group-owner.png', fullPage: true })

    // The customer, no password yet: the link signs in, the guest page offers (not forces) a password.
    const customer = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1280, height: 900 } })
    const guest = await customer.newPage()
    await fixLocale(guest)
    await guest.goto(link)
    await guest.waitForURL(/\/messagehub\/guest\?/)
    await expect(guest.getByTestId('sso-first-password')).toBeVisible()
    await guest.getByTestId('guest-accept').click()
    await expect(guest.getByTestId('guest-conversation')).toBeVisible()
    const side = guest.getByTestId('guest-side-panel')
    await expect(side.getByRole('tab', { name: '客服工单' })).toBeVisible()
    await expect(side.getByTestId('group-panel-frame')).toHaveAttribute('data-state', 'ready', { timeout: 20_000 })
    const frameSrc = new URL((await side.locator('iframe').getAttribute('src'))!)
    expect(frameSrc.origin).toBe(appOrigin)
    expect(frameSrc.pathname).toBe('/support/panel')
    const composer = guest.getByPlaceholder(/^(Message|输入消息)/).first()
    await composer.fill(`客户留言 ${runId}`)
    await composer.press('Enter')
    await expect(guest.getByText(`客户留言 ${runId}`).first()).toBeVisible()
    await guest.screenshot({ path: 'test-results/dv-support-group-guest.png', fullPage: true })

    // Set a password from the prompt; the current terminal stays signed in.
    await guest.getByTestId('sso-first-password-open').click()
    await guest.getByLabel('New password').fill(`ui-pass-${runId}`)
    await guest.getByLabel('Confirm password').fill(`ui-pass-${runId}`)
    await guest.getByTestId('sso-first-password-save').click()
    await expect(guest.getByTestId('sso-first-password-done')).toBeVisible()
    const userId = (await guest.getByTestId('sso-first-password-done').textContent())!.match(/as (\S+) with/)?.[1]
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
    await rpc(request, 'msg-center', 'group.delete', { group_did: groupDid }, token).catch(() => {})
  }
})

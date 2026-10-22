import { createHash } from 'node:crypto'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'

/**
 * Zone display settings, profile pictures and their propagation on a running zone with the
 * control-panel and msg-center of this tree:
 *
 *   BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=… \
 *     pnpm exec playwright test --config=playwright.dv.config.ts object-presentation
 *
 * The browser test signs in through the real login page, so the base URL must be the Zone root
 * origin: control-panel only delivers system session tokens there, not to `sys.<zone>`.
 */
type JsonRecord = Record<string, unknown>

const adminUser = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const adminPassword = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || ''
const PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEUlEQVR4nGN4VmGDFTEMLQkAN6BmgaudJikAAAAASUVORK5CYII='

function hashPassword(username: string, password: string, nonce: number): string {
  const original = createHash('sha256').update(`${password}${username}.buckyos`, 'utf8').digest('base64')
  return createHash('sha256').update(`${original}${nonce}`, 'utf8').digest('base64')
}

async function rpc(request: APIRequestContext, baseURL: string, service: string, method: string, params: JsonRecord, token?: string): Promise<JsonRecord> {
  const seq = Date.now()
  const response = await request.post(`${baseURL}/kapi/${service}`, { data: { method, params, sys: token ? [seq, token] : [seq] }, ignoreHTTPSErrors: true })
  if (!response.ok()) throw new Error(`${method} HTTP ${response.status()}`)
  const body = await response.json() as JsonRecord
  if (body.error) throw new Error(`${method}: ${JSON.stringify(body.error)}`)
  return (body.result ?? {}) as JsonRecord
}

async function sessions(request: APIRequestContext, baseURL: string) {
  const nonce = Date.now()
  const login = await rpc(request, baseURL, 'control-panel', 'auth.login', {
    username: adminUser,
    password: hashPassword(adminUser, adminPassword, nonce),
    appid: 'control-panel',
    target: { kind: 'system', service_id: 'control-panel' },
    login_nonce: nonce,
  })
  const sudoNonce = nonce + 1
  const sudo = await rpc(request, baseURL, 'verify-hub', 'sudo_by_password', {
    username: adminUser,
    password: hashPassword(adminUser, adminPassword, sudoNonce),
    target: { kind: 'system', service_id: 'control-panel' },
    aud: 'system-config',
    login_nonce: sudoNonce,
  })
  return { token: String(login.session_token), sudoToken: String(sudo.session_token) }
}

const PNG_FILE = Buffer.from(PNG.split(',')[1], 'base64')
const favicon = (page: Page) => page.locator('link[rel~="icon"]').getAttribute('href')

async function loginThroughUi(page: Page) {
  await page.goto('/')
  await expect(page.getByLabel('Username')).toBeVisible()
  await page.getByLabel('Username').fill(adminUser)
  await page.getByLabel('Password').fill(adminPassword)
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  await expect(page.getByRole('button', { name: 'BuckyOS' }).first()).toBeVisible({ timeout: 30_000 })
}

test.describe('object presentation on the DV zone', () => {
  test.skip(!adminPassword, 'Set BUCKYOS_TEST_ADMIN_PASSWORD for the zone administrator.')

  test('Zone display settings: public read, sudo-only write, per-field clear, sign-in page', async ({ page, request, baseURL }) => {
    const { token, sudoToken } = await sessions(request, baseURL!)
    const anonymous = await rpc(request, baseURL!, 'control-panel', 'zone.presentation.get', {})
    expect(String(anonymous.zone_did)).toMatch(/^did:/)
    await expect(rpc(request, baseURL!, 'control-panel', 'zone.presentation.set', { display_name: 'x' }, token)).rejects.toThrow(/sudo/i)
    await expect(rpc(request, baseURL!, 'control-panel', 'zone.presentation.set', { icon: 'https://example.com/i.png' }, sudoToken)).rejects.toThrow(/icon/i)
    try {
      await rpc(request, baseURL!, 'control-panel', 'zone.presentation.set', { display_name: '我的家庭云', description: 'DV check', icon: PNG }, sudoToken)
      const stored = await rpc(request, baseURL!, 'control-panel', 'zone.presentation.get', {})
      expect(stored.settings).toEqual({ display_name: '我的家庭云', description: 'DV check', icon: PNG })
      const cleared = await rpc(request, baseURL!, 'control-panel', 'zone.presentation.set', { description: null }, sudoToken)
      expect(cleared.settings).toEqual({ display_name: '我的家庭云', icon: PNG })

      await page.goto('/login')
      await expect(page.getByRole('heading', { name: 'Sign in to 我的家庭云' })).toBeVisible()
      await expect(page).toHaveTitle('Sign in · 我的家庭云')
      await expect.poll(() => page.locator('link[rel~="icon"]').getAttribute('href')).toBe(PNG)
    } finally {
      await rpc(request, baseURL!, 'control-panel', 'zone.presentation.set', { display_name: null, description: null, icon: null }, sudoToken)
    }
  })

  test('a profile picture is read back, listed publicly, synced to contacts and withdrawn when cleared', async ({ request, baseURL }) => {
    const { token } = await sessions(request, baseURL!)
    await expect(rpc(request, baseURL!, 'control-panel', 'user.profile.set', { avatar: 'https://example.com/a.png' }, token)).rejects.toThrow(/avatar/i)
    const ownContact = async (did: string) => {
      const contacts = await rpc(request, baseURL!, 'msg-center', 'contact.list_contacts', { query: {}, contact_mgr_owner: did }, token) as unknown as JsonRecord[]
      return contacts.find((contact) => contact.did === did)
    }
    try {
      await rpc(request, baseURL!, 'control-panel', 'user.profile.set', { avatar: PNG }, token)
      const detail = await rpc(request, baseURL!, 'control-panel', 'user.get', { user_id: adminUser }, token)
      expect((detail.profile as JsonRecord).avatar).toBe(PNG)
      const list = await rpc(request, baseURL!, 'control-panel', 'user.list', {}, token)
      const me = (list.users as JsonRecord[]).find((user) => user.user_id === adminUser)!
      expect(me.avatar).toBe(PNG)
      const did = String(me.did)
      expect((await ownContact(did))?.avatar).toBe(PNG)

      await rpc(request, baseURL!, 'control-panel', 'user.profile.set', { avatar: null }, token)
      expect(((await rpc(request, baseURL!, 'control-panel', 'user.get', { user_id: adminUser }, token)).profile as JsonRecord).avatar).toBeUndefined()
      expect((await ownContact(did))?.avatar).toBeUndefined()
    } finally {
      await rpc(request, baseURL!, 'control-panel', 'user.profile.set', { avatar: null }, token)
    }
  })

  test('browser: the Desktop shows the Zone brand, an uploaded picture reaches the taskbar, and MessageHub shows another user\'s picture', async ({ page, request, baseURL }) => {
    const { token, sudoToken } = await sessions(request, baseURL!)
    const peer = 'alice'
    try {
      await rpc(request, baseURL!, 'control-panel', 'zone.presentation.set', { display_name: '我的家庭云', icon: PNG }, sudoToken)
      // Another user's profile is written with the administrator's sudo session.
      await rpc(request, baseURL!, 'control-panel', 'user.profile.set', { user_id: peer, avatar: PNG }, sudoToken)
      await loginThroughUi(page)
      await expect(page).toHaveTitle('我的家庭云')
      await expect.poll(() => favicon(page)).toBe(PNG)

      const avatarButton = page.getByRole('button', { name: 'BuckyOS' }).first()
      await avatarButton.hover()
      await page.getByTestId('taskbar-profile-card').getByRole('button', { name: 'Manage account' }).click()
      const win = page.getByTestId('window-users-agents')
      await win.getByRole('button', { name: 'Avatar' }).click()
      const dialog = page.getByRole('dialog', { name: 'Profile picture' })
      await dialog.getByTestId('self-avatar-input').setInputFiles({ name: 'me.png', mimeType: 'image/png', buffer: PNG_FILE })
      await dialog.getByTestId('self-avatar-save').click()
      await expect(dialog).toHaveCount(0)
      await expect(avatarButton.locator('img')).toHaveAttribute('src', /^data:image\/jpeg;base64,/)
      await expect(win.locator('img[src^="data:image/jpeg"]').first()).toBeVisible()

      await page.reload()
      await expect(page.getByRole('button', { name: 'BuckyOS' }).first().locator('img')).toHaveAttribute('src', /^data:image\/jpeg;base64,/, { timeout: 30_000 })

      const hub = await page.context().newPage()
      await hub.goto('/messagehub')
      await expect(hub).toHaveTitle('MessageHub · 我的家庭云')
      // The peer's own picture shows on their contact row (the owner's own entry is not listed).
      const peerAvatar = hub.locator(`img[src="${PNG}"]`).first()
      await expect(peerAvatar).toBeVisible({ timeout: 30_000 })
      await expect(peerAvatar.locator('xpath=ancestor::*[contains(translate(normalize-space(.), "ALICE", "alice"), "alice")][1]')).toHaveCount(1)
      await hub.close()
    } finally {
      await rpc(request, baseURL!, 'control-panel', 'user.profile.set', { avatar: null }, token)
      await rpc(request, baseURL!, 'control-panel', 'user.profile.set', { user_id: peer, avatar: null }, sudoToken)
      await rpc(request, baseURL!, 'control-panel', 'zone.presentation.set', { display_name: null, icon: null }, sudoToken)
    }
  })

  test('browser: HomeStation shows the owner\'s picture and the zone feed takes the Zone display settings', async ({ page, request, baseURL }) => {
    const { token, sudoToken } = await sessions(request, baseURL!)
    try {
      await rpc(request, baseURL!, 'control-panel', 'user.profile.set', { avatar: PNG }, token)
      await rpc(request, baseURL!, 'control-panel', 'zone.presentation.set', { display_name: '我的家庭云', icon: PNG }, sudoToken)
      await loginThroughUi(page)
      // Profile → msg-center contact sync → HomeStation identities (its contact cache lives 30s).
      await page.goto('/homestation')
      await expect.poll(async () => {
        await page.reload()
        return page.getByTestId('hs-avatar').locator('img').getAttribute('src', { timeout: 15_000 }).catch(() => null)
      }, { timeout: 90_000, intervals: [5_000] }).toBe(PNG)
      // The service refreshes the Zone's display name and icon every minute.
      await expect.poll(async () => (await rpc(request, baseURL!, 'homestation', 'portal.home', {})).zoneName, { timeout: 90_000, intervals: [5_000] }).toBe('我的家庭云')
      expect((await rpc(request, baseURL!, 'homestation', 'portal.home', {})).zoneIcon).toBe(PNG)
      await page.goto('/homestation/~zone')
      await expect(page.getByTestId('hs-portal-zone')).toContainText('我的家庭云')
      await expect(page.getByTestId('hs-portal-zone').locator('img')).toHaveAttribute('src', PNG)
    } finally {
      await rpc(request, baseURL!, 'control-panel', 'user.profile.set', { avatar: null }, token)
      await rpc(request, baseURL!, 'control-panel', 'zone.presentation.set', { display_name: null, icon: null }, sudoToken)
    }
  })

  test('app summaries carry document text and no unserved icon path', async ({ request, baseURL }) => {
    const { token } = await sessions(request, baseURL!)
    const apps = await rpc(request, baseURL!, 'control-panel', 'apps.list', {}, token)
    for (const app of apps.apps as JsonRecord[]) {
      expect(app).toHaveProperty('presentation')
      expect(app).toHaveProperty('name')
      expect(app).not.toHaveProperty('icon_res_url')
    }
  })
})

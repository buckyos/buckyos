import { createHash, generateKeyPairSync, sign } from 'node:crypto'
import { expect, test, type APIRequestContext, type Locator, type Page } from '@playwright/test'

/**
 * My Network against a running zone with the msg-center, homestation and Desktop of this tree
 * (notepads/my-network-upgrade-todo.md §6.9):
 *
 *   BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io BUCKYOS_TEST_ADMIN_PASSWORD=bucky2025 \
 *     pnpm exec playwright test --config=playwright.dv.config.ts my-network
 *
 * Signs in as the member user (`bob` of the `start.py --all` seed, password `buckyos2026`). All
 * users of one Zone are friends already, so friend request round trips are not part of this run.
 */
type JsonRecord = Record<string, unknown>

const adminUser = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const adminPassword = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || ''
const memberUser = process.env.BUCKYOS_TEST_MEMBER_USER || 'bob'
const memberPassword = process.env.BUCKYOS_TEST_MEMBER_PASSWORD || 'buckyos2026'
const stamp = Date.now().toString(36)

function hashPassword(username: string, password: string, nonce: number): string {
  const original = createHash('sha256').update(`${password}${username}.buckyos`, 'utf8').digest('base64')
  return createHash('sha256').update(`${original}${nonce}`, 'utf8').digest('base64')
}

let seq = Date.now()
async function rpc(request: APIRequestContext, baseURL: string, service: string, method: string, params: JsonRecord, token?: string): Promise<JsonRecord> {
  seq += 1
  const response = await request.post(`${baseURL}/kapi/${service}`, { data: { method, params, sys: token ? [seq, token] : [seq] }, ignoreHTTPSErrors: true })
  if (!response.ok()) throw new Error(`${method} HTTP ${response.status()}`)
  const body = await response.json() as JsonRecord
  if (body.error) throw new Error(`${method}: ${JSON.stringify(body.error)}`)
  return (body.result ?? {}) as JsonRecord
}

async function login(request: APIRequestContext, baseURL: string, username: string, password: string): Promise<string> {
  const nonce = Date.now()
  const result = await rpc(request, baseURL, 'control-panel', 'auth.login', {
    username,
    password: hashPassword(username, password, nonce),
    appid: 'control-panel',
    target: { kind: 'system', service_id: 'control-panel' },
    login_nonce: nonce,
  })
  return String(result.session_token)
}

async function loginThroughUi(page: Page, username: string, password: string) {
  await page.goto('/')
  await expect(page.getByLabel('Username')).toBeVisible()
  await page.getByLabel('Username').fill(username)
  await page.getByLabel('Password').fill(password)
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  await expect(page.getByRole('button', { name: 'BuckyOS' }).first()).toBeVisible({ timeout: 30_000 })
}

/** RFC 8785 canonical JSON of plain data (no floats), the input of a named object id. */
function jcs(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(jcs).join(',')}]`
  if (value && typeof value === 'object') {
    const record = value as JsonRecord
    return `{${Object.keys(record).sort().map((key) => `${JSON.stringify(key)}:${jcs(record[key])}`).join(',')}}`
  }
  return JSON.stringify(value)
}

/** A friend request signed by a fresh `did:dev` identity, as another Zone sends it. */
function signedFriendRequest(to: string, name: string, message: string) {
  const { publicKey, privateKey } = generateKeyPairSync('ed25519')
  const x = String((publicKey.export({ format: 'jwk' }) as JsonRecord).x)
  const from = `did:dev:${x}`
  const now = Date.now()
  const claims = {
    from,
    to: [to],
    kind: 'operation',
    created_at_ms: now,
    content: {
      format: 'text/plain',
      content: message,
      machine: {
        intent: 'buckyos.friend',
        data: { action: 'request', request_id: `dv-${now.toString(36)}`, message, profile_hint: { name }, expires_at_ms: now + 7 * 86_400_000 },
      },
    },
  }
  const header = Buffer.from(JSON.stringify({ alg: 'EdDSA', kid: `${from}#main_key` })).toString('base64url')
  const payload = Buffer.from(JSON.stringify(claims)).toString('base64url')
  const signature = sign(null, Buffer.from(`${header}.${payload}`), privateKey).toString('base64url')
  return { from, objId: `cymsg:${createHash('sha256').update(jcs(claims)).digest('hex')}`, jwt: `${header}.${payload}.${signature}` }
}

async function openMyNetwork(page: Page): Promise<Locator> {
  await page.getByTestId('desktop-app-my-network').click()
  const win = page.getByTestId('window-my-network')
  await expect(win.getByTestId('my-network-home')).toBeVisible({ timeout: 30_000 })
  return win
}

test.describe('My Network on the DV zone', () => {
  test.skip(!adminPassword, 'Set BUCKYOS_TEST_ADMIN_PASSWORD for the zone administrator.')
  test.use({ viewport: { width: 1440, height: 900 } })

  test('real relations, collections, lookup, groups and the MessageHub hand-off', async ({ page, request, baseURL }) => {
    const admin = await login(request, baseURL!, adminUser, adminPassword)
    const users = (await rpc(request, baseURL!, 'control-panel', 'user.list', {}, admin)).users as JsonRecord[]
    const didOf = (id: string) => String(users.find((u) => u.user_id === id && typeof u.did === 'string')?.did)
    const adminDid = didOf(adminUser)
    const memberDid = didOf(memberUser)
    const member = await login(request, baseURL!, memberUser, memberPassword)
    const adminContact = await rpc(request, baseURL!, 'msg-center', 'contact.get_contact', { did: adminDid, contact_mgr_owner: memberDid }, member)
    const adminName = String(adminContact.name)

    // The admin hosts two groups: the member is invited into one (same-Zone friends are
    // accepted at once) and joins the other through an invite link from the UI.
    const invited = await rpc(request, baseURL!, 'msg-center', 'group.create', {
      idempotency_key: `mn-dv-a-${stamp}`,
      profile: { name: `MN DV invited ${stamp}` },
      invitations: [{ member_did: memberDid }],
    }, admin)
    const linked = await rpc(request, baseURL!, 'msg-center', 'group.create', {
      idempotency_key: `mn-dv-b-${stamp}`,
      profile: { name: `MN DV link ${stamp}` },
      invitations: [],
    }, admin)
    const groups = [String(invited.group_did), String(linked.group_did)]
    const link = await rpc(request, baseURL!, 'msg-center', 'group.create_invite_link', {
      group_did: groups[1],
      idempotency_key: `mn-dv-link-${stamp}`,
    }, admin)
    const collection = `DV ${stamp}`

    try {
      const errors: string[] = []
      page.on('pageerror', error => errors.push(String(error)))
      await loginThroughUi(page, memberUser, memberPassword)
      const win = await openMyNetwork(page)

      // Real contacts: the Zone's users are friends; nothing reports a load error.
      await win.getByTestId('nav-friends').click()
      const admins = win.getByTestId('my-network-person').filter({ has: page.getByRole('button', { name: adminName, exact: true }) })
      await expect(admins).toHaveCount(1)
      await expect(win.getByTestId('my-network-person').filter({ hasText: memberDid })).toHaveCount(0)
      await expect(admins.getByTestId('relation-friend')).toBeVisible()
      await expect(win.getByTestId('my-network-list-error')).toHaveCount(0)
      await expect(win.getByTestId('nav-collection-zone_user')).toBeVisible()

      // Add friend: a Zone user is already a friend; an unknown DID says why it cannot be found.
      await win.getByTestId('my-network-add-friend').click()
      let dialog = win.getByRole('dialog', { name: 'Add Friend / Contact' })
      await dialog.getByLabel('Find people').fill(adminDid)
      const row = dialog.getByTestId('find-people-result').filter({ hasText: adminName }).first()
      await row.getByRole('button', { name: 'View', exact: true }).click()
      await expect(dialog.getByTestId('add-friend-already')).toBeVisible()
      await dialog.getByRole('button', { name: 'Back', exact: true }).click()
      await dialog.getByLabel('Find people').fill('did:bns:nobody-mn-dv')
      await expect(dialog.getByTestId('find-people-lookup-error')).toBeVisible({ timeout: 20_000 })
      await dialog.getByRole('button', { name: 'Close dialog' }).click()

      // A user collection is a tag written through msg-center.
      await win.getByTestId('my-network-create-collection').click()
      const nameDialog = win.getByRole('dialog', { name: 'New collection' })
      await nameDialog.getByLabel('Collection name').fill(collection)
      await nameDialog.getByRole('button', { name: 'Choose members' }).click()
      const members = win.getByRole('dialog', { name: `Add people to “${collection}”` })
      await members.getByRole('button', { name: adminName, exact: true }).click()
      await members.getByRole('button', { name: 'Create collection' }).click()
      await expect(win.getByTestId('my-network-list-title')).toHaveText(collection)
      await expect(win.getByTestId('my-network-person')).toHaveCount(1)
      const tagged = await rpc(request, baseURL!, 'msg-center', 'contact.get_contact', { did: adminDid, contact_mgr_owner: memberDid }, member)
      expect(tagged.tags).toContain(collection)
      await win.getByTestId('collection-delete').click()
      await win.getByRole('dialog').filter({ hasText: collection }).getByTestId('network-confirm-submit').click()
      await expect(win.getByTestId(`nav-collection-${collection}`)).toHaveCount(0)

      // Groups: the invited one is already joined; the other is joined through its link.
      await win.getByTestId('nav-groups').click()
      await expect(win.getByTestId('my-network-group').filter({ hasText: `MN DV invited ${stamp}` })).toBeVisible()
      await win.getByTestId('my-network-join-group').click()
      dialog = win.getByRole('dialog', { name: 'Join Group' })
      await dialog.getByLabel('Group DID or invite link').fill(`${groups[1]}?invite=${String(link.token)}`)
      await dialog.getByRole('button', { name: 'Find', exact: true }).click()
      await expect(dialog.getByTestId('join-group-preview')).toContainText(`MN DV link ${stamp}`)
      await dialog.getByTestId('join-group-submit').click()
      await expect(dialog.getByTestId('join-group-outcome')).toHaveAttribute('data-outcome', 'active')
      await dialog.getByRole('button', { name: 'Done', exact: true }).click()
      await expect(win.getByTestId('group-detail')).toHaveAttribute('data-did', groups[1])
      await expect(win.getByTestId('group-detail')).toContainText('Hosted by this Zone')

      // Requests load for real (nothing pending is fine).
      await win.getByTestId('nav-requests').click()
      const requests = win.getByTestId('my-network-requests')
      await expect(requests.locator('[data-testid="requests-empty"], section[data-testid^="requests-"]').first()).toBeVisible()
      await expect(requests.getByTestId('requests-error')).toHaveCount(0)
      await expect(requests.getByTestId('requests-partial')).toHaveCount(0)

      // "Send message" opens the conversation in MessageHub.
      await win.getByTestId('nav-friends').click()
      await admins.getByRole('button').first().click()
      await win.getByTestId('contact-send-message').click()
      await expect(page.getByTestId('window-messagehub')).toBeVisible({ timeout: 30_000 })
      expect(errors).toEqual([])
    } finally {
      for (const group of groups) {
        await rpc(request, baseURL!, 'msg-center', 'group.delete', { group_did: group, idempotency_key: `mn-dv-del-${group}` }, admin).catch(() => undefined)
      }
    }
  })

  test('a friend request from another Zone arrives signed, shows in Requests and is accepted in MessageHub', async ({ page, request, baseURL }) => {
    const admin = await login(request, baseURL!, adminUser, adminPassword)
    const users = (await rpc(request, baseURL!, 'control-panel', 'user.list', {}, admin)).users as JsonRecord[]
    const memberDid = String(users.find((u) => u.user_id === memberUser && typeof u.did === 'string')?.did)
    const name = `DV Stranger ${stamp}`
    const text = `Hello from another Zone ${stamp}`
    const signed = signedFriendRequest(memberDid, name, text)

    // No Bearer token: the signature is the only credential; the gateway forwards /kapi/msg-center/.
    const put = (body: string, contentType: string, objId?: string) => request.put(`${baseURL}/kapi/msg-center/${memberDid}/inbox`, {
      headers: { 'content-type': contentType, ...(objId ? { 'cyfs-obj-id': objId } : {}) },
      data: body,
      ignoreHTTPSErrors: true,
    })
    const accepted = await put(signed.jwt, 'application/cyfs-named-object+jwt', signed.objId)
    expect(accepted.status()).toBe(200)
    expect(accepted.headers()['cyfs-dispatch-status']).toBe('accepted')
    expect((await put(signed.jwt, 'application/cyfs-named-object+jwt', signed.objId)).status()).toBe(200)
    expect((await put('{}', 'application/cyfs-named-object+json')).status()).toBe(401)

    const member = await login(request, baseURL!, memberUser, memberPassword)
    try {
      await loginThroughUi(page, memberUser, memberPassword)
      const win = await openMyNetwork(page)
      await win.getByTestId('nav-requests').click()
      const incoming = win.getByTestId('request-friend-incoming').filter({ hasText: name })
      await expect(incoming).toHaveCount(1)
      await expect(incoming).toContainText(text)

      await win.getByTestId('nav-contacts').click()
      await win.getByRole('button', { name, exact: true }).click()
      await expect(win.getByTestId('contact-detail').getByTestId('relation-incoming_pending')).toBeVisible()
      await win.getByTestId('contact-send-message').click()
      const hub = page.getByTestId('window-messagehub')
      await expect(hub).toBeVisible({ timeout: 30_000 })
      const card = hub.getByTestId('friend-notice')
      await expect(card).toHaveAttribute('data-state', 'pending', { timeout: 30_000 })
      await expect(card).toContainText(text)
      await card.getByRole('button', { name: 'Accept' }).click()
      await expect(card).toHaveAttribute('data-state', 'accepted')

      const contact = await rpc(request, baseURL!, 'msg-center', 'contact.get_contact', { did: signed.from, contact_mgr_owner: memberDid }, member)
      expect(contact.friend_state).toBe('friend')
      expect(contact.access_level).toBe('allow')
    } finally {
      await rpc(request, baseURL!, 'msg-center', 'contact.remove_contact', { did: signed.from, contact_mgr_owner: memberDid }, member).catch(() => undefined)
    }
  })
})

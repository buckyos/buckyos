// Cross-Zone DID federation in a real browser against the `ssotest` VM group
// (src/dev_configs/ssotest.json; notepads/done/hybrid-sso-implementation-todo.md SSO-09 / SSO-14): the
// owner of the canonical Zone signs in to the target Zone with its DID through the canonical consent
// page, declines once, and reaches an AI Workspace entry on the target that allows DID login. The
// host does not use the SN for DNS, so the browser and the Node-side kRPC calls take the Zone
// addresses from BUCKYOS_TEST_HOST_RULES ("MAP <host> <ip>, ..."). Use the SN IP to exercise relay
// paths, including Alice behind NAT and Charlie with a custom domain. Defaults: canonical
// bob.web3.devtests.org (wan_dyn), target charlie.me (portmap); BUCKYOS_TEST_CANONICAL[_USER] and
// BUCKYOS_TEST_TARGET[_USER] pick others. Run with the two VM IPs:
//   BUCKYOS_TEST_HOST_RULES="MAP bob.web3.devtests.org <ip>, MAP charlie.me <ip>" \
//   pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/hybrid-sso-federation.spec.ts
import { createHash, randomUUID } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import https from 'node:https'
import type { LookupFunction } from 'node:net'
import { expect, type Page, test } from '@playwright/test'
import { PROTOCOL_VERSION } from '../../src/app/aiworkspace/api/types'

type JsonRecord = Record<string, unknown>

const hostRules = process.env.BUCKYOS_TEST_HOST_RULES?.trim() ?? ''
const homeZone = process.env.BUCKYOS_TEST_CANONICAL || 'bob.web3.devtests.org'
const homeUser = process.env.BUCKYOS_TEST_CANONICAL_USER || 'bob'
const resourceZone = process.env.BUCKYOS_TEST_TARGET || 'charlie.me'
const resourceUser = process.env.BUCKYOS_TEST_TARGET_USER || 'charlie'
const password = process.env.BUCKYOS_TEST_PASSWORD || 'bucky2025'
const homeDid = `did:bns:${homeUser}`
const runId = Date.now().toString(36)

const addresses = new Map(
  hostRules.split(',').map((rule) => rule.trim().split(/\s+/)).filter((parts) =>
    parts[0] === 'MAP' && parts.length === 3
  ).map((parts) => [parts[1], parts[2]] as const),
)

test.skip(!addresses.has(homeZone) || !addresses.has(resourceZone), 'BUCKYOS_TEST_HOST_RULES must map both Zones')
const workspaceHostPattern = `aiws-*.${resourceZone}`
const workspaceAddress = addresses.get(workspaceHostPattern) ?? addresses.get(resourceZone)
const extraHome = process.env.BUCKYOS_TEST_OTHER_HOME || 'alice.web3.devtests.org'
const extraMap = addresses.has(extraHome) ? ', MAP aiws-*.' + extraHome + ' ' + addresses.get(extraHome) : ''
const mappedRules = hostRules + extraMap +
  (workspaceAddress ? ', MAP sys.' + resourceZone + ' ' + workspaceAddress : '')
const browserRules = workspaceAddress && !addresses.has(workspaceHostPattern)
  ? `${mappedRules}, MAP ${workspaceHostPattern} ${workspaceAddress}`
  : mappedRules
test.use({
  trace: 'off',
  screenshot: 'off',
  video: 'off',
  ignoreHTTPSErrors: true,
  launchOptions: {
    args: [
      `--host-resolver-rules=${browserRules}`,
      ...(process.env.BUCKYOS_TEST_HTTP1 === '1' ? ['--disable-http2'] : []),
    ],
  },
})

const lookup: LookupFunction = (hostname, options, callback) => {
  const address = addresses.get(hostname)
  if (!address) return callback(new Error(`no address for ${hostname}`), '', 4)
  if (options.all) {
    return (callback as (e: null, a: { address: string; family: number }[]) => void)(null, [{ address, family: 4 }])
  }
  callback(null, address, 4)
}

function post(host: string, path: string, body: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const request = https.request(`https://${host}${path}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      lookup,
      ca: readFileSync(process.env.BUCKYOS_TEST_CA || homedir() + '/buckycli/ca/buckyos_test_ca_ca_cert.pem'),
    }, (response) => {
      const chunks: Buffer[] = []
      response.on('data', (chunk: Buffer) => chunks.push(chunk))
      response.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')))
    })
    request.on('error', reject)
    request.end(body)
  })
}

let seq = Date.now()
async function rpc(
  host: string,
  service: string,
  method: string,
  params: JsonRecord,
  token?: string,
): Promise<JsonRecord> {
  seq = Math.max(Date.now(), seq + 1)
  const body = JSON.parse(
    await post(host, `/kapi/${service}`, JSON.stringify({ method, params, sys: token ? [seq, token] : [seq] })),
  ) as JsonRecord
  if (body.error) throw new Error(`${method}: ${String(body.error)} ${JSON.stringify(body.data ?? null)}`)
  const result = (body.result ?? {}) as JsonRecord
  if (result.ok === false && result.error) throw new Error(`${method}: ${JSON.stringify(result.error)}`)
  return result
}

function hashPassword(username: string, plain: string, nonce?: number): string {
  const original = createHash('sha256').update(`${plain}${username}.buckyos`, 'utf8').digest('base64')
  if (nonce === undefined) return original
  return createHash('sha256').update(`${original}${nonce}`, 'utf8').digest('base64')
}

async function ownerToken(host: string, username: string): Promise<string> {
  const nonce = Date.now()
  const result = await rpc(host, 'control-panel', 'auth.login', {
    username,
    password: hashPassword(username, password, nonce),
    target: { kind: 'system', service_id: 'control-panel' },
    login_nonce: nonce,
  })
  return String(result.session_token)
}

/** From the target's DID form to the canonical consent page, signing in there on the way. */
async function reachConsent(
  page: Page,
  targetHost = resourceZone,
  user = homeUser,
  host = homeZone,
  allowRemembered = false,
) {
  const did = `did:bns:${user}`
  const didField = page.getByTestId('sso-did-input')
  if (await didField.isEnabled()) await didField.fill(did)
  else await expect(didField).toHaveValue(did)
  await page.getByTestId('sso-did-submit').click()
  await page.waitForURL((url) => url.host === host)
  // Without a session there the consent page sends the browser to the canonical sign-in first.
  const username = page.getByLabel('Username')
  const consent = page.getByTestId('sso-federate-consent')
  await expect(username.or(consent)).toBeVisible()
  if (await username.isVisible()) {
    await username.fill(user)
    await page.getByLabel('Password').fill(password)
    await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  }
  if (allowRemembered) {
    await expect(consent.or(page.getByTestId('desktop-taskbar'))).toBeVisible()
    if (new URL(page.url()).host === targetHost) return false
  }
  await page.waitForURL((url) => url.host === host && url.pathname === '/sso/federate')
  await expect(consent).toBeVisible()
  await expect(page.getByTestId('sso-federate-did')).toHaveText(did)
  await expect(page.getByTestId('sso-federate-target')).toHaveText(targetHost)
  await expect(page.getByTestId('sso-federate-risk')).toContainText(targetHost)
  return true
}

test('declining at the canonical Zone returns to the target without a session', async ({ page }) => {
  await page.goto(`https://${resourceZone}/login`)
  await page.getByTestId('login-use-did').click()
  await reachConsent(page)
  await page.getByTestId('sso-federate-deny').click()
  await page.waitForURL((url) => url.host === resourceZone && url.pathname === '/login')
  expect(new URL(page.url()).searchParams.get('did_error')).toBe('access_denied')
  await expect(page.getByTestId('login-did-error')).toBeVisible()
  const cookies = await page.context().cookies(`https://${resourceZone}`)
  expect(cookies.find((cookie) => cookie.name === 'buckyos_session_token')).toBeUndefined()
})

test('a DID from another Zone joins an editor group and edits on the workspace origin', async ({ page, browser }) => {
  const owner = await ownerToken(resourceZone, resourceUser)
  const title = `sso-${runId}`
  const ws = await rpc(resourceZone, 'aiworkspace', 'ws.create', { title }, owner)
  const workspaceId = String(ws.workspace_id)
  const workspaceHost = `aiws-${workspaceId.replace(/^ws_/, '')}.${resourceZone}`
  if (workspaceAddress && !addresses.has(workspaceHost)) addresses.set(workspaceHost, workspaceAddress)
  const at = { workspace_id: workspaceId }
  const editor = ['read', 'append', 'update', 'delete', 'structure', 'comment']
  const groupId = 'editors'
  await rpc(resourceZone, 'aiworkspace', 'acl.set_group', { ...at, group_id: groupId, title: 'Editors' }, owner)
  await rpc(resourceZone, 'aiworkspace', 'acl.set', {
    ...at,
    changes: [{ subject: `@${groupId}`, resource: '/', allow: editor }],
  }, owner)
  const deepLink = `https://${workspaceHost}/${title}?mode=edit#comments`
  const launch = await rpc(resourceZone, 'control-panel', 'auth.prepare_app_launch', {
    service_id: 'aiworkspace',
    redirect_url: deepLink,
  }, owner)
  expect(launch.status).toBe('ready')
  const ownerContext = await browser.newContext({ ignoreHTTPSErrors: true })
  const ownerPage = await ownerContext.newPage()
  await ownerPage.goto(String(launch.callback_url))
  // The collaboration toolbar becomes available once the workspace has a canvas.
  await ownerPage.getByRole('button', { name: 'New free canvas' }).click()
  await ownerPage.goto(deepLink)
  await ownerPage.getByTestId('aiws-share').click()
  await ownerPage.getByTestId('aiws-share-tab-entries').click()
  await ownerPage.getByTestId('aiws-share-entry-did_login').check()
  await ownerPage.getByTestId('aiws-share-audience').selectOption('directed')
  await ownerPage.getByTestId('aiws-share-recipient').fill(homeDid)
  await ownerPage.getByTestId('aiws-share-entry-preset').selectOption('@' + groupId)
  await ownerPage.getByTestId('aiws-share-entry-create').click()
  const input = ownerPage.getByTestId('aiws-share-entry-link').first()
  await expect(input).toHaveValue(/\/sso\/share\/inv-/)
  const inviteUrl = await input.inputValue()
  await ownerPage.getByTestId('aiws-share-entry-copy').first().click()
  const directory = await rpc(resourceZone, 'control-panel', 'user.invite.directory', {}, owner)
  const recipient = (directory.users as JsonRecord[]).find((person) => person.did === homeDid)
  expect(
    (recipient?.sources as JsonRecord[])?.some((src) =>
      src.invite_id === new URL(inviteUrl).pathname.split('/').pop()
    ),
  ).toBe(true)
  await ownerPage.goto('https://' + resourceZone + '/login?redirect_url=' + encodeURIComponent(inviteUrl))
  await ownerPage.getByLabel('Username').fill(resourceUser)
  await ownerPage.getByLabel('Password').fill(password)
  await ownerPage.getByRole('button', { name: 'Sign In', exact: true }).click()
  await expect(ownerPage.getByTestId('sso-share-page')).toContainText('This invitation is for a different identity.')
  await expect(ownerPage.getByRole('button', { name: /Accept as/ })).toHaveCount(0)
  await ownerPage.getByRole('link', { name: 'Cancel', exact: true }).click()
  await ownerContext.close()

  await page.goto(inviteUrl)
  await expect(page.getByTestId('sso-share-page')).toContainText(homeDid)
  await reachConsent(page, workspaceHost)
  await page.getByTestId('sso-federate-remember').getByRole('checkbox').uncheck()
  await page.getByTestId('sso-federate-approve').click()
  await page.getByRole('button', { name: `Accept as ${homeDid}`, exact: true }).click()
  await page.waitForURL((url) => url.host === workspaceHost && url.pathname !== '/sso_callback')
  await expect(page).toHaveURL(deepLink)
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', workspaceId)
  await expect(page.getByTestId('aiws-principal')).toHaveAttribute('data-principal', homeDid)

  const workspaceIdentity = await page.evaluate(async () => {
    const response = await fetch('/sso_refresh', { method: 'POST', credentials: 'same-origin' })
    if (!response.ok) throw new Error(`Workspace session refresh returned ${response.status}`)
    return (await response.json()).identity
  })
  expect(workspaceIdentity).toMatchObject({
    kind: 'external',
    did: homeDid,
    auth_method: 'did_federation',
    restricted: true,
  })
  expect(workspaceIdentity.scopes).toContain('aiworkspace')
  const workspaceSession = (await page.context().cookies(`https://${workspaceHost}`))
    .find((cookie) => cookie.domain === workspaceHost && cookie.name === 'buckyos_session_token')
  expect(workspaceSession).toBeDefined()
  const collaborator = workspaceSession!.value
  const info = await rpc(workspaceHost, 'aiworkspace', 'ws.get_info', at, collaborator)
  expect(info.principal).toBe(homeDid)
  expect(new Set(info.capabilities as string[])).toEqual(new Set(editor))
  const field = { field_id: 'title', name: 'Title', type: 'text' }
  const saved = await rpc(workspaceHost, 'aiworkspace', 'doc.commit', {
    ...at,
    epoch: info.epoch,
    protocol_version: PROTOCOL_VERSION,
    session_id: 'dv-federation',
    idempotency_key: runId,
    operations: [{
      op: 'entity.create',
      entity_id: 'shared-table',
      type_id: 'buckyos.table-source',
      parent_id: 'data',
      order_key: 'a',
      payload: { fields: [field] },
    }],
  }, collaborator)
  expect(saved.status).toBe('accepted')
  const read = await rpc(resourceZone, 'aiworkspace', 'doc.read', { ...at, entity_id: 'shared-table' }, owner)
  expect(read.content).toMatchObject({ fields: [field] })
  const privateWs = await rpc(resourceZone, 'aiworkspace', 'ws.create', { title: 'private-' + runId }, owner)
  await expect(rpc(workspaceHost, 'aiworkspace', 'doc.read', {
    workspace_id: privateWs.workspace_id,
    entity_id: 'data',
  }, collaborator)).rejects.toThrow()

  const session = (await page.context().cookies(`https://${resourceZone}`)).find((cookie) =>
    cookie.name === 'buckyos_session_token'
  )
  expect(session).toBeUndefined()
  await page.goto(deepLink)
  await expect(page.getByTestId('aiws-principal')).toHaveAttribute('data-principal', homeDid)
  await page.goto(`https://sys.${resourceZone}/login?redirect_url=${encodeURIComponent(deepLink)}`)
  await expect(page).toHaveURL(inviteUrl)
  await page.goto(`https://${resourceZone}/`)
  await expect(page).toHaveURL(/\/login(?:\?|$)/)
  await expect(page.getByTestId('desktop-taskbar')).toHaveCount(0)
  await page.getByTestId('login-use-did').click()
  await reachConsent(page)
  await page.getByTestId('sso-federate-remember').getByRole('checkbox').uncheck()
  await page.getByTestId('sso-federate-approve').click()
  await expect(page.getByTestId('login-did-error')).toBeVisible()
  expect((await page.context().cookies('https://' + resourceZone)).some((c) => c.name === 'buckyos_session_token'))
    .toBe(false)
  const share = await rpc(resourceZone, 'control-panel', 'auth.share.describe', {
    invite_id: new URL(inviteUrl).pathname.split('/').pop(),
  })
  // Re-adding Bob as an ordinary group member must not revive the earlier invitation's session.
  await rpc(resourceZone, 'aiworkspace', 'acl.members', { ...at, group_id: groupId, remove: [homeDid] }, owner)
  await rpc(resourceZone, 'aiworkspace', 'acl.members', { ...at, group_id: groupId, add: [homeDid] }, owner)
  await expect(rpc(workspaceHost, 'aiworkspace', 'doc.read', { ...at, entity_id: 'shared-table' }, collaborator))
    .rejects.toThrow()
  await rpc(resourceZone, 'aiworkspace', 'ws.close_entry', {
    ...at,
    entry_id: share.resource_id,
    remove_members: true,
  }, owner)
  await expect(rpc(workspaceHost, 'aiworkspace', 'doc.read', { ...at, entity_id: 'shared-table' }, collaborator))
    .rejects.toThrow()
  await page.goto('https://' + workspaceHost + '/favicon.ico')
  expect(await page.evaluate(async () => (await fetch('/sso_refresh', { method: 'POST' })).ok)).toBe(false)
})

test('an open workspace invitation retains a distinct grant for each native DID', async ({ browser }) => {
  const otherHost = process.env.BUCKYOS_TEST_OTHER_HOME || 'alice.web3.devtests.org'
  const otherUser = process.env.BUCKYOS_TEST_OTHER_USER || 'alice'
  test.skip(!addresses.has(otherHost), 'Map the second home Zone to verify two independent recipients')
  const owner = await ownerToken(resourceZone, resourceUser)
  const title = 'open-' + runId
  const ws = await rpc(resourceZone, 'aiworkspace', 'ws.create', { title }, owner)
  const at = { workspace_id: ws.workspace_id }
  await rpc(resourceZone, 'aiworkspace', 'acl.set_group', { ...at, group_id: 'readers', title: 'Readers' }, owner)
  await rpc(resourceZone, 'aiworkspace', 'acl.set', {
    ...at,
    changes: [{ subject: '@readers', resource: '/', allow: ['read'] }],
  }, owner)
  const result = await rpc(resourceZone, 'aiworkspace', 'ws.issue_share', {
    ...at,
    group_id: 'readers',
    request_id: randomUUID(),
    audience: { kind: 'open' },
    methods: { did_login: true },
    return_path: '/' + title,
  }, owner)
  const invitation = new URL(String((result.entry as JsonRecord).share_url))
  const id = invitation.pathname.split('/').pop()
  const target = 'aiws-' + String(ws.workspace_id).replace(/^ws_/, '') + '.' + resourceZone
  addresses.set(target, workspaceAddress!)
  const grants = async () => {
    const directory = await rpc(resourceZone, 'control-panel', 'user.invite.directory', {}, owner)
    return (directory.users as JsonRecord[]).filter((row) =>
      (row.sources as JsonRecord[]).some((src) => src.invite_id === id)
    )
      .map((row) => row.did).sort()
  }
  expect(await grants()).toEqual([])
  for (const [user, host] of [[homeUser, homeZone], [otherUser, otherHost]]) {
    const context = await browser.newContext({ ignoreHTTPSErrors: true })
    try {
      const page = await context.newPage()
      await page.goto(invitation.href)
      await reachConsent(page, target, user, host)
      await page.getByTestId('sso-federate-remember').getByRole('checkbox').uncheck()
      await page.getByTestId('sso-federate-approve').click()
      await page.getByRole('button', { name: 'Accept as did:bns:' + user, exact: true }).click()
      await page.waitForURL((url) => url.host === target && url.pathname === '/' + title)
      const token = (await context.cookies('https://' + target)).find((c) =>
        c.name === 'buckyos_session_token'
      )!.value
      const info = await rpc(target, 'aiworkspace', 'ws.get_info', at, token)
      expect(info.principal).toBe('did:bns:' + user)
      expect(info.capabilities).toEqual(['read'])
    } finally {
      await context.close()
    }
  }
  expect(await grants()).toEqual([homeDid, 'did:bns:' + otherUser].sort())
})

// Run after test_user_invite_dv.ts has enrolled the native Charlie identity in Alice.
test('a system-qualified DID gains access to a private workspace only after accepting its grant', async ({ page }) => {
  test.skip(process.env.BUCKYOS_TEST_ENROLLED !== '1', 'Run the three-Zone native enrollment DV first')
  const zone = process.env.BUCKYOS_TEST_OTHER_HOME || 'alice.web3.devtests.org'
  const ownerUser = process.env.BUCKYOS_TEST_OTHER_USER || 'alice'
  const owner = await ownerToken(zone, ownerUser)
  const title = 'enrolled-' + runId
  const ws = await rpc(zone, 'aiworkspace', 'ws.create', { title }, owner)
  const at = { workspace_id: ws.workspace_id }
  const target = 'aiws-' + String(at.workspace_id).replace(/^ws_/, '') + '.' + zone
  addresses.set(target, addresses.get(zone)!)
  await rpc(zone, 'aiworkspace', 'acl.set_group', { ...at, group_id: 'readers', title: 'Readers' }, owner)
  await rpc(zone, 'aiworkspace', 'acl.set', {
    ...at,
    changes: [{ subject: '@readers', resource: '/', allow: ['read'] }],
  }, owner)
  await page.goto('https://' + zone + '/login')
  await page.getByTestId('login-use-did').click()
  if (await reachConsent(page, zone, resourceUser, resourceZone, true)) {
    await page.getByTestId('sso-federate-remember').getByRole('checkbox').uncheck()
    await page.getByTestId('sso-federate-approve').click()
  }
  await page.waitForURL((u) => u.host === zone && u.pathname === '/')
  const rootToken = async () =>
    (await page.context().cookies('https://' + zone)).find((c) => c.name === 'buckyos_session_token')?.value
  const session = await rootToken()
  expect(session).toBeTruthy()
  const identity = await rpc(zone, 'control-panel', 'auth.current_identity', {}, session)
  expect(identity.kind).toBe('account')
  await expect(rpc(zone, 'aiworkspace', 'doc.read', { ...at, entity_id: 'data' }, session)).rejects.toThrow()
  const share = await rpc(zone, 'aiworkspace', 'ws.issue_share', {
    ...at,
    request_id: randomUUID(),
    group_id: 'readers',
    audience: { kind: 'directed', did: 'did:bns:' + resourceUser },
    methods: { did_login: true },
    return_path: '/' + title,
  }, owner)
  await page.goto(String((share.entry as JsonRecord).share_url))
  await page.getByRole('button', { name: 'Accept as ' + String(identity.did), exact: true }).click()
  await page.waitForURL((u) => u.host === target && u.pathname === '/' + title)
  const token = (await page.context().cookies('https://' + target)).find((c) => c.name === 'buckyos_session_token')
    ?.value
  expect(token).toBeTruthy()
  const info = await rpc(target, 'aiworkspace', 'ws.get_info', at, token)
  expect(info.capabilities).toEqual(['read'])
  expect((await rpc(zone, 'control-panel', 'auth.current_identity', {}, await rootToken())).did).toBe(identity.did)
})

test('the home Zone remembers two workspaces as one service and prompts again after withdrawal', async ({ browser }) => {
  const local = 'consent-' + runId
  const nonce = Date.now()
  const approval = await rpc(homeZone, 'verify-hub', 'sudo_by_password', {
    username: homeUser,
    password: hashPassword(homeUser, password, nonce),
    login_nonce: nonce,
    target: { kind: 'system', service_id: 'control-panel' },
    aud: 'system-config',
  })
  await rpc(homeZone, 'control-panel', 'user.create', {
    user_id: local,
    user_type: 'user',
    password_hash: hashPassword(local, password),
  }, String(approval.session_token))
  const user = await ownerToken(homeZone, local)
  const me = await rpc(homeZone, 'control-panel', 'auth.current_identity', {}, user)
  const owner = await ownerToken(resourceZone, resourceUser)
  const context = await browser.newContext({ ignoreHTTPSErrors: true })
  try {
    const page = await context.newPage()
    const consent = async (n: string) => {
      const ws = await rpc(resourceZone, 'aiworkspace', 'ws.create', { title: 'consent-' + runId + '-' + n }, owner)
      const at = { workspace_id: ws.workspace_id }
      await rpc(resourceZone, 'aiworkspace', 'acl.set_group', { ...at, group_id: 'readers', title: 'Readers' }, owner)
      await rpc(resourceZone, 'aiworkspace', 'acl.set', {
        ...at,
        changes: [{ subject: '@readers', resource: '/', allow: ['read'] }],
      }, owner)
      const grant = await rpc(resourceZone, 'aiworkspace', 'ws.issue_share', {
        ...at,
        group_id: 'readers',
        audience: { kind: 'directed', did: me.did },
        methods: { did_login: true },
        request_id: randomUUID(),
      }, owner)
      const invite = new URL(String((grant.entry as JsonRecord).share_url))
      const begun = await rpc(resourceZone, 'control-panel', 'auth.did.begin', {
        did: me.did,
        purpose: 'share',
        invite_id: invite.pathname.split('/').pop(),
        redirect_url: invite.href,
      })
      const response = await page.goto(String(begun.url))
      expect(response?.ok()).toBe(true)
      // The home Zone redirects its anonymous browser to /login with the signed federation URL.
      let url = new URL(page.url())
      if (url.pathname === '/login') url = new URL(url.searchParams.get('redirect_url')!)
      expect(url.host).toBe(homeZone)
      expect(url.pathname).toBe('/sso/federate')
      const fields = Object.fromEntries(url.searchParams)
      const description = await rpc(homeZone, 'control-panel', 'auth.federation.describe', fields, user)
      expect(description.app_host).toBe('aiws-' + String(at.workspace_id).replace(/^ws_/, '') + '.' + resourceZone)
      return { fields, description }
    }
    const first = await consent('first')
    expect(first.description.already_granted).toBe(false)
    await rpc(homeZone, 'control-panel', 'auth.federation.approve', { ...first.fields, remember: true }, user)
    const second = await consent('second')
    expect(second.description.already_granted).toBe(true)
    const grants = await rpc(homeZone, 'control-panel', 'auth.federation.consents', {}, user)
    const list = grants.consents as JsonRecord[]
    expect(list).toHaveLength(1)
    await rpc(homeZone, 'control-panel', 'auth.federation.forget', { id: list[0].id }, user)
    expect((await rpc(homeZone, 'control-panel', 'auth.federation.describe', second.fields, user)).already_granted)
      .toBe(false)
  } finally {
    await context.close()
  }
})

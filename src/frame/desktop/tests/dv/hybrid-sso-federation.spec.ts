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
import { createHash } from 'node:crypto'
import https from 'node:https'
import type { LookupFunction } from 'node:net'
import { expect, test, type Page } from '@playwright/test'
import { PROTOCOL_VERSION } from '../../src/app/aiworkspace/api/types'

type JsonRecord = Record<string, unknown>

const hostRules = process.env.BUCKYOS_TEST_HOST_RULES?.trim() ?? ''
const zoneA = process.env.BUCKYOS_TEST_CANONICAL || 'bob.web3.devtests.org'
const userA = process.env.BUCKYOS_TEST_CANONICAL_USER || 'bob'
const zoneB = process.env.BUCKYOS_TEST_TARGET || 'charlie.me'
const userB = process.env.BUCKYOS_TEST_TARGET_USER || 'charlie'
const password = process.env.BUCKYOS_TEST_PASSWORD || 'bucky2025'
const didA = `did:bns:${userA}`
const runId = Date.now().toString(36)

const addresses = new Map(
  hostRules.split(',').map((rule) => rule.trim().split(/\s+/)).filter((parts) => parts[0] === 'MAP' && parts.length === 3).map((parts) => [parts[1], parts[2]] as const),
)

test.skip(!addresses.has(zoneA) || !addresses.has(zoneB), 'BUCKYOS_TEST_HOST_RULES must map both Zones')
const workspaceHostPattern = `aiws-*.${zoneB}`
const workspaceAddress = addresses.get(workspaceHostPattern) ?? addresses.get(zoneB)
const browserRules = workspaceAddress && !addresses.has(workspaceHostPattern)
  ? `${hostRules}, MAP ${workspaceHostPattern} ${workspaceAddress}`
  : hostRules
test.use({ ignoreHTTPSErrors: true, launchOptions: { args: [`--host-resolver-rules=${browserRules}`] } })

const lookup: LookupFunction = (hostname, options, callback) => {
  const address = addresses.get(hostname)
  if (!address) return callback(new Error(`no address for ${hostname}`), '', 4)
  if (options.all) return (callback as (e: null, a: { address: string; family: number }[]) => void)(null, [{ address, family: 4 }])
  callback(null, address, 4)
}

function post(host: string, path: string, body: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const request = https.request(`https://${host}${path}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      lookup,
      rejectUnauthorized: false,
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
async function rpc(host: string, service: string, method: string, params: JsonRecord, token?: string): Promise<JsonRecord> {
  seq = Math.max(Date.now(), seq + 1)
  const body = JSON.parse(await post(host, `/kapi/${service}`, JSON.stringify({ method, params, sys: token ? [seq, token] : [seq] }))) as JsonRecord
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
async function reachConsent(page: Page) {
  await page.getByTestId('sso-did-input').fill(didA)
  await page.getByTestId('sso-did-submit').click()
  await page.waitForURL((url) => url.host === zoneA)
  // Without a session there the consent page sends the browser to the canonical sign-in first.
  const username = page.getByLabel('Username')
  const consent = page.getByTestId('sso-federate-consent')
  await expect(username.or(consent)).toBeVisible()
  if (await username.isVisible()) {
    await username.fill(userA)
    await page.getByLabel('Password').fill(password)
    await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  }
  await page.waitForURL((url) => url.host === zoneA && url.pathname === '/sso/federate')
  await expect(consent).toBeVisible()
  await expect(page.getByTestId('sso-federate-did')).toHaveText(didA)
  await expect(page.getByTestId('sso-federate-target')).toContainText(zoneB)
}

test('declining at the canonical Zone returns to the target without a session', async ({ page }) => {
  await page.goto(`https://${zoneB}/login`)
  await page.getByTestId('login-use-did').click()
  await reachConsent(page)
  await page.getByTestId('sso-federate-deny').click()
  await page.waitForURL((url) => url.host === zoneB && url.pathname === '/login')
  expect(new URL(page.url()).searchParams.get('did_error')).toBe('access_denied')
  await expect(page.getByTestId('login-did-error')).toBeVisible()
  const cookies = await page.context().cookies(`https://${zoneB}`)
  expect(cookies.find((cookie) => cookie.name === 'buckyos_session_token')).toBeUndefined()
})

test('a DID from another Zone joins an editor group and edits on the workspace origin', async ({ page }) => {
  const owner = await ownerToken(zoneB, userB)
  const ws = await rpc(zoneB, 'aiworkspace', 'ws.create', { title: `联合入口 ${runId}` }, owner)
  const workspaceId = String(ws.workspace_id)
  const workspaceHost = `aiws-${workspaceId.replace(/^ws_/, '')}.${zoneB}`
  if (workspaceAddress && !addresses.has(workspaceHost)) addresses.set(workspaceHost, workspaceAddress)
  const at = { workspace_id: workspaceId }
  const editor = ['read', 'append', 'update', 'delete', 'structure', 'comment']
  const groupId = 'editors'
  await rpc(zoneB, 'aiworkspace', 'acl.set_group', { ...at, group_id: groupId, title: 'Editors' }, owner)
  await rpc(zoneB, 'aiworkspace', 'acl.set', { ...at, changes: [{ subject: `@${groupId}`, resource: '/', allow: editor }] }, owner)
  const entry = await rpc(zoneB, 'aiworkspace', 'ws.create_entry', {
    ...at,
    group_id: groupId,
    methods: { guest: false, password: false, self_register: false, did_login: true },
  }, owner)
  const entryId = String((entry.entry as JsonRecord).entry_id)

  await page.goto(`https://${zoneB}/sso/entry?service=aiworkspace&entry=${encodeURIComponent(entryId)}`)
  await expect(page.getByTestId('sso-entry-method-guest')).toHaveCount(0)
  await page.getByTestId('sso-entry-method-did_login').click()
  await reachConsent(page)
  await page.getByTestId('sso-federate-approve').click()

  // Back on the target's entry page as the External identity the DID brought along: the join the
  // visitor chose before leaving runs by itself and opens the workspace.
  await page.waitForURL((url) => url.host === workspaceHost && url.pathname !== '/sso_callback')
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', workspaceId)
  await expect(page.getByTestId('aiws-principal')).toHaveAttribute('data-principal', didA)

  const workspaceIdentity = await page.evaluate(async () => {
    const response = await fetch('/sso_refresh', { method: 'POST', credentials: 'same-origin' })
    if (!response.ok) throw new Error(`Workspace session refresh returned ${response.status}`)
    return (await response.json()).identity
  })
  expect(workspaceIdentity).toMatchObject({ kind: 'external', did: didA, auth_method: 'did_federation', restricted: true })
  expect(workspaceIdentity.scopes).toContain('aiworkspace')
  const workspaceSession = (await page.context().cookies(`https://${workspaceHost}`))
    .find((cookie) => cookie.domain === workspaceHost && cookie.name === 'buckyos_session_token')
  expect(workspaceSession).toBeDefined()
  const collaborator = workspaceSession!.value
  const info = await rpc(workspaceHost, 'aiworkspace', 'ws.get_info', at, collaborator)
  expect(info.principal).toBe(didA)
  expect(new Set(info.capabilities as string[])).toEqual(new Set(editor))
  const field = { field_id: 'title', name: 'Title', type: 'text' }
  const saved = await rpc(workspaceHost, 'aiworkspace', 'doc.commit', {
    ...at, epoch: info.epoch, protocol_version: PROTOCOL_VERSION, session_id: 'dv-federation', idempotency_key: runId,
    operations: [{ op: 'entity.create', entity_id: 'shared-table', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'a', payload: { fields: [field] } }],
  }, collaborator)
  expect(saved.status).toBe('accepted')
  const read = await rpc(zoneB, 'aiworkspace', 'doc.read', { ...at, entity_id: 'shared-table' }, owner)
  expect(read.content).toMatchObject({ fields: [field] })

  const session = (await page.context().cookies(`https://${zoneB}`)).find((cookie) => cookie.name === 'buckyos_session_token')
  expect(session).toBeTruthy()
  const identity = await rpc(zoneB, 'control-panel', 'auth.current_identity', {}, session!.value)
  expect(identity).toMatchObject({ kind: 'external', did: didA, auth_method: 'did_federation', restricted: true })

  // The External identity gets the restricted portal at the target's root, not the Desktop.
  await page.goto(`https://${zoneB}/`)
  await expect(page.getByTestId('restricted-home')).toBeVisible()
  await expect(page.getByTestId('desktop-taskbar')).toHaveCount(0)
})

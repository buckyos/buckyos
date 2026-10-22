// Cross-Zone DID federation in a real browser against the `ssotest` VM group
// (src/dev_configs/ssotest.json; notepads/hybrid-sso-implementation-todo.md SSO-09 / SSO-14): the
// owner of the canonical Zone signs in to the target Zone with its DID through the canonical consent
// page, declines once, and reaches an AI Workspace entry on the target that allows DID login. The
// host does not use the SN for DNS, so the browser and the Node-side kRPC calls take the Zone
// addresses from BUCKYOS_TEST_HOST_RULES ("MAP <host> <ip>, ..."). Pick Zones the browser reaches
// directly: the SN relay rate-limits the many connections a cold SPA load opens. Defaults: canonical
// bob.web3.devtests.org (wan_dyn), target charlie.me (portmap); BUCKYOS_TEST_CANONICAL[_USER] and
// BUCKYOS_TEST_TARGET[_USER] pick others. Run with the two VM IPs:
//   BUCKYOS_TEST_HOST_RULES="MAP bob.web3.devtests.org <ip>, MAP charlie.me <ip>" \
//   pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/hybrid-sso-federation.spec.ts
import { createHash } from 'node:crypto'
import https from 'node:https'
import type { LookupFunction } from 'node:net'
import { expect, test, type Page } from '@playwright/test'

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
test.use({ ignoreHTTPSErrors: true, launchOptions: { args: [`--host-resolver-rules=${hostRules}`] } })

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

test('a DID from another Zone reaches an AI Workspace entry on the target', async ({ page }) => {
  const owner = await ownerToken(zoneB, userB)
  const ws = await rpc(zoneB, 'aiworkspace', 'ws.create', { title: `联合入口 ${runId}` }, owner)
  const workspaceId = String(ws.workspace_id)
  const entry = await rpc(zoneB, 'aiworkspace', 'ws.create_entry', {
    workspace_id: workspaceId,
    capabilities: ['read', 'comment'],
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
  await page.waitForURL(new RegExp(`^https://${zoneB.replaceAll('.', '\\.')}/workspace/${workspaceId}`))
  await expect(page.getByText(`联合入口 ${runId}`)).toBeVisible()
  await expect(page.getByTestId('aiws-open-error')).toHaveCount(0)

  const session = (await page.context().cookies(`https://${zoneB}`)).find((cookie) => cookie.name === 'buckyos_session_token')
  expect(session).toBeTruthy()
  const identity = await rpc(zoneB, 'control-panel', 'auth.current_identity', {}, session!.value)
  expect(identity).toMatchObject({ kind: 'external', did: didA, auth_method: 'did_federation', restricted: true })

  // The External identity gets the restricted portal at the target's root, not the Desktop.
  await page.goto(`https://${zoneB}/`)
  await expect(page.getByTestId('restricted-home')).toBeVisible()
  await expect(page.getByTestId('desktop-taskbar')).toHaveCount(0)
})

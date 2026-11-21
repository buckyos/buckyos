import { createHash, randomBytes } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import https from 'node:https'
import type { LookupFunction } from 'node:net'
import { expect, test, type Page } from '@playwright/test'
import { PROTOCOL_VERSION } from '../../src/app/aiworkspace/api/types'

type Row = Record<string, unknown>
const env = process.env
const enabled = env.BUCKYOS_DV_OAUTH === '1'
const home = env.BUCKYOS_DV_OAUTH_ZONE || 'charlie.me'
const remote = env.BUCKYOS_DV_OAUTH_REMOTE || 'bob.web3.devtests.org'
const homeAdmin = env.BUCKYOS_DV_OAUTH_ADMIN || 'charlie'
const remoteAdmin = env.BUCKYOS_DV_OAUTH_REMOTE_ADMIN || 'bob'
const rules = env.BUCKYOS_TEST_HOST_RULES || ''
const googleState = env.BUCKYOS_DV_GOOGLE_STORAGE_STATE
const googleEmail = env.BUCKYOS_DV_GOOGLE_TEST_EMAIL
const run = randomBytes(6).toString('hex')
const providers = [
  { kind: 'oidc', origin: env.BUCKYOS_DV_OIDC_ORIGIN || 'https://oidc.sso-dv.test:8443' },
  { kind: 'oauth2', origin: env.BUCKYOS_DV_OAUTH2_ORIGIN || 'https://oauth2.sso-dv.test:8443' },
]
const fixtureDid = (origin: string) => 'did:web:e-h2hmn7ceaxsvpet3wy4mwvexzz24q7ho.' + new URL(origin).hostname
const zoneOf = (token: string) => JSON.parse(Buffer.from(token.split('.')[1], 'base64url').toString()).zone
const resolver = 'sys_config_resolve_oauth_did'
const hosts = new Map(
  rules
    .split(',')
    .map((s) => s.trim().split(/\s+/))
    .filter((parts) => parts.length === 3 && parts[0] === 'MAP')
    .map(([, k, v]) => [k, v]),
)
const lookup: LookupFunction = (host, opts, cb) => {
  const ip = hosts.get(host) ?? hosts.get(host.replace(/^aiws-[^.]+\./, 'aiws-*.'))
  if (!ip) return cb(new Error('Missing DV DNS mapping for ' + host), '', 4)
  if (opts.all)
    return (cb as (e: null, items: { address: string; family: number }[]) => void)(null, [
      { address: ip, family: 4 },
    ])
  cb(null, ip, 4)
}
const browserRules = [
  rules,
  ...[home, remote]
    .filter((h) => hosts.has(h) && !hosts.has('aiws-*.' + h))
    .map((h) => 'MAP aiws-*.' + h + ' ' + hosts.get(h)),
].join(', ')
test.use({
  trace: 'off',
  screenshot: 'off',
  video: 'off',
  launchOptions: {
    args: [
      '--host-resolver-rules=' + browserRules,
      '--ignore-certificate-errors',
      ...(env.BUCKYOS_TEST_HTTP1 === '1' ? ['--disable-http2'] : []),
    ],
  },
})
test.skip(!enabled, 'set BUCKYOS_DV_OAUTH=1 to use the isolated DV Zones')
test.describe.configure({ timeout: 120000 })
let seq = Date.now()
function rpc(host: string, service: string, method: string, params: Row, token?: string): Promise<Row> {
  const ca = readFileSync(env.BUCKYOS_TEST_CA || homedir() + '/buckycli/ca/buckyos_test_ca_ca_cert.pem')
  const id = ++seq
  return new Promise((resolve, reject) => {
    const req = https.request(
      'https://' + host + '/kapi/' + service,
      { method: 'POST', lookup, ca, timeout: 20000, headers: { 'Content-Type': 'application/json' } },
      (res) => {
        const chunks: Buffer[] = []
        res.on('data', (chunk) => chunks.push(chunk))
        res.on('end', () => {
          try {
            const reply = JSON.parse(Buffer.concat(chunks).toString())
            if (res.statusCode !== 200 || reply.error || reply.result?.ok === false) {
              const reason =
                reply.data?.reason ??
                reply.result?.error?.detail ??
                reply.result?.error?.message ??
                reply.error ??
                res.statusCode
              reject(new Error(method + ' failed: ' + String(reason)))
            } else resolve(reply.result)
          } catch {
            reject(new Error(method + ' returned invalid JSON'))
          }
        })
      },
    )
    req.on('error', reject)
    req.on('timeout', () => req.destroy(new Error('DV Zone timed out')))
    req.end(JSON.stringify({ method, params, sys: token ? [id, token] : [id] }))
  })
}
const digest = (s: string) => createHash('sha256').update(s).digest('base64')
async function login(host: string, user: string, password: string) {
  const nonce = Date.now()
  const result = await rpc(host, 'control-panel', 'auth.login', {
    username: user,
    password: digest(digest(password + user + '.buckyos') + nonce),
    login_nonce: nonce,
    target: { kind: 'system', service_id: 'control-panel' },
  })
  return String(result.session_token)
}
async function owner(host: string) {
  return login(host, host === home ? homeAdmin : remoteAdmin, env.BUCKYOS_TEST_PASSWORD || 'bucky2025')
}
async function sudoForCreate() {
  const nonce = Date.now(),
    password = env.BUCKYOS_TEST_PASSWORD || 'bucky2025'
  const result = await rpc(home, 'verify-hub', 'sudo_by_password', {
    username: homeAdmin,
    password: digest(digest(password + homeAdmin + '.buckyos') + nonce),
    login_nonce: nonce,
    target: { kind: 'system', service_id: 'control-panel' },
    aud: 'system-config',
  })
  return String(result.session_token)
}
const caps = ['read', 'append', 'update', 'delete', 'structure', 'comment']
async function workspace(host: string) {
  const token = await owner(host)
  const title = 'oauth-' + run
  const r = await rpc(host, 'aiworkspace', 'ws.create', { title }, token)
  const workspace_id = String(r.workspace_id)
  await rpc(host, 'aiworkspace', 'acl.set_group', { workspace_id, group_id: 'editors', title: 'Editors' }, token)
  await rpc(
    host,
    'aiworkspace',
    'acl.set',
    { workspace_id, changes: [{ subject: '@editors', resource: '/', allow: caps }] },
    token,
  )
  return { host, workspace_id, token, path: '/' + title }
}
async function issue(
  ws: Awaited<ReturnType<typeof workspace>>,
  audience: Row,
  email?: { source_id: string; email: string },
) {
  const result = await rpc(
    ws.host,
    'aiworkspace',
    'ws.issue_share',
    {
      workspace_id: ws.workspace_id,
      group_id: 'editors',
      request_id: randomBytes(8).toString('hex'),
      audience,
      recipient_email: email,
      methods: { oauth: true, did_login: true, password: true },
      return_path: ws.path + '?view=edit#comments',
    },
    ws.token,
  )
  const url = String((result.entry as Row).share_url)
  expect(new URL(url).origin).toBe('https://' + ws.host)
  return url
}
async function fixedLocale(page: Page) {
  await page.addInitScript(() => localStorage.setItem('buckyos.prototype.locale.v1', 'en'))
}
async function openProvider(page: Page, id: string, provider: string) {
  await page.getByRole('button', { name: 'Continue with ' + id, exact: true }).click()
  await page.waitForURL((u) => u.origin === provider && u.pathname === '/authorize')
}
async function providerLogin(page: Page, id: string, provider: string, person = 'alice') {
  await openProvider(page, id, provider)
  await page.getByLabel('Test identity').selectOption(person)
  await page.getByRole('button', { name: 'Continue test sign-in' }).click()
  await page.waitForURL((u) => u.hostname === home && !u.pathname.startsWith('/sso/oauth/'))
}
async function verifyApp(page: Page, ws: Awaited<ReturnType<typeof workspace>>, subject: string, method: string) {
  const host = 'aiws-' + ws.workspace_id.replace(/^ws_/, '') + '.' + ws.host
  const ip = hosts.get(host) ?? hosts.get('aiws-*.' + ws.host) ?? hosts.get(ws.host)
  if (ip) hosts.set(host, ip)
  await page.waitForURL(
    (u) => u.hostname === host && u.pathname === ws.path && u.search === '?view=edit' && u.hash === '#comments',
  )
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', ws.workspace_id)
  const cookies = await page.context().cookies('https://' + host)
  const token = cookies.find((c) => c.name === 'buckyos_session_token' && c.domain === host)?.value
  if (!token) throw new Error('Target host-only session missing')
  const claims = JSON.parse(Buffer.from(token.split('.')[1], 'base64url').toString())
  expect(claims.auth_method).toBe(method)
  if (method === 'oauth') expect(claims.oauth_source_id).toBeTruthy()
  const at = { workspace_id: ws.workspace_id }
  const info = await rpc(host, 'aiworkspace', 'ws.get_info', at, token)
  expect(info.principal).toBe(subject)
  const entity = 'dv-' + randomBytes(6).toString('hex')
  const fields = [{ field_id: 'title', name: 'Title', type: 'text' }]
  const saved = await rpc(
    host,
    'aiworkspace',
    'doc.commit',
    {
      ...at,
      epoch: info.epoch,
      protocol_version: PROTOCOL_VERSION,
      session_id: 'dv-oauth',
      idempotency_key: entity,
      operations: [
        {
          op: 'entity.create',
          entity_id: entity,
          type_id: 'buckyos.table-source',
          parent_id: 'data',
          order_key: 'a',
          payload: { fields },
        },
      ],
    },
    token,
  )
  expect(saved.status).toBe('accepted')
  const read = await rpc(ws.host, 'aiworkspace', 'doc.read', { ...at, entity_id: entity }, ws.token)
  expect(read.content).toMatchObject({ fields })
}

const changedSources = new Map<string, boolean>()
async function settings(token: string) {
  const current = await rpc(home, 'control-panel', 'sso.settings.get', {}, token)
  return (current.oauth_sources ?? []) as Row[]
}
async function setFixtureSource(token: string, id: string, enabled: boolean) {
  const current = await settings(token)
  const source = current.find((row) => row.id === id)
  if (
    !source ||
    source.client_id !== id ||
    !providers.some((p) => 'dv-' + p.kind === id && source.authorization_endpoint === p.origin + '/authorize')
  ) {
    throw new Error('Dedicated fixture source required')
  }
  await rpc(
    home,
    'control-panel',
    'sso.settings.set',
    {
      oauth_sources: current.map((row) => (row.id === id ? { ...row, enabled } : row)),
    },
    token,
  )
}
test.beforeAll(async () => {
  const token = await owner(home)
  const current = await settings(token)
  const updates = [...current]
  for (const { kind, origin } of providers) {
    const url = new URL(origin)
    if (!url.hostname.endsWith('.test')) throw new Error('Dedicated .test provider required')
    const id = 'dv-' + kind
    const existing = current.find((p) => p.id === id)
    if (
      existing &&
      (existing.client_id !== id ||
        existing.authorization_endpoint !== origin + '/authorize' ||
        existing.webhost !== url.hostname)
    ) {
      throw new Error('Existing source does not belong to this fixture')
    }
    if (existing?.enabled) continue
    if (env.BUCKYOS_DV_OAUTH_CONFIGURE !== '1')
      throw new Error('Preconfigure dedicated test sources or set BUCKYOS_DV_OAUTH_CONFIGURE=1 on a test Zone')
    const source = {
      id,
      display_name: id,
      kind,
      enabled: true,
      webhost: url.hostname,
      client_id: id,
      secret_ref: 'buckyos-dv',
      authorization_endpoint: origin + '/authorize',
      token_endpoint: origin + '/token',
      ...(kind === 'oidc'
        ? { issuer: origin, jwks_uri: origin + '/keys' }
        : {
            account_endpoint: origin + '/me',
            id_field: 'sub',
            email_field: 'email',
            email_verified_field: 'email_verified',
          }),
    }
    if (existing) updates[updates.indexOf(existing)] = { ...existing, enabled: true }
    else updates.push(source)
    changedSources.set(id, false)
  }
  if (changedSources.size) await rpc(home, 'control-panel', 'sso.settings.set', { oauth_sources: updates }, token)
})
test.afterAll(async () => {
  if (!changedSources.size) return
  const token = await owner(home)
  const current = await settings(token)
  await rpc(
    home,
    'control-panel',
    'sso.settings.set',
    { oauth_sources: current.map((p) => (changedSources.has(String(p.id)) ? { ...p, enabled: false } : p)) },
    token,
  )
})

for (const { kind, origin } of providers) {
  test(kind + ' system login handles consent cancellation and an unbound identity', async ({ page }) => {
    const source = 'dv-' + kind
    await fixedLocale(page)
    await page.goto('https://' + home + '/login')
    await page.getByRole('button', { name: 'Continue with ' + source, exact: true }).click()
    await page.waitForURL((u) => u.origin === origin && u.pathname === '/authorize')
    await page.getByRole('button', { name: 'Cancel', exact: true }).click()
    await page.waitForURL((u) => u.hostname === home && u.searchParams.get('did_error') === 'access_denied')
    await page.goto('https://' + home + '/login')
    await providerLogin(page, source, origin)
    expect(new URL(page.url()).searchParams.get('did_error')).toBe('oauth_account_unbound')
    expect((await page.context().cookies('https://' + home)).some((c) => c.name === 'buckyos_session_token')).toBe(
      false,
    )
  })
  test(kind + ' directed share verifies the account and delivers the scoped editor session', async ({ page }) => {
    const source = 'dv-' + kind
    const ws = await workspace(home)
    const invitation = new URL(
      await issue(ws, { kind: 'directed' }, { source_id: source, email: 'Alice@Company.test' }),
    )
    invitation.searchParams.set('from', 'shared-link')
    invitation.hash = 'details'
    const url = invitation.href
    const share = await rpc(home, 'control-panel', 'auth.share.describe', {
      invite_id: new URL(url).pathname.split('/').pop(),
    })
    const did = String((share.audience as Row).did)
    expect(did).toBe(fixtureDid(origin))
    await fixedLocale(page)
    await page.goto(url)
    const assertInvitationError = async (reason: string) => {
      await page.waitForURL(
        (u) =>
          u.origin === new URL(url).origin &&
          u.pathname === invitation.pathname &&
          u.searchParams.get('from') === 'shared-link' &&
          u.hash === '#details' &&
          u.searchParams.get('did_error') === reason,
      )
      await expect(page.getByRole('alert').filter({ hasText: 'Sign-in did not complete' })).toBeVisible()
      const proof = await page.evaluate(async (inviteId) => {
        const result = await fetch('/kapi/control-panel', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            method: 'auth.share.identity',
            params: { invite_id: inviteId },
            sys: [Date.now()],
          }),
        })
        return (await result.json()).result?.oauth
      }, share.invite_id)
      expect(proof).toBeNull()
      expect(
        (await page.context().cookies('https://' + home)).some((c) => c.name === 'buckyos_session_token'),
      ).toBe(false)
    }
    await openProvider(page, source, origin)
    await page.getByRole('button', { name: 'Cancel', exact: true }).click()
    await assertInvitationError('access_denied')
    if (env.BUCKYOS_DV_OAUTH_CONFIGURE === '1') {
      await openProvider(page, source, origin)
      await setFixtureSource(ws.token, source, false)
      try {
        await page.getByRole('button', { name: 'Continue test sign-in' }).click()
        await assertInvitationError('oauth_source_disabled')
        await expect(page.getByRole('button', { name: 'Continue with ' + source })).toHaveCount(0)
      } finally {
        await setFixtureSource(ws.token, source, true)
      }
      await page.reload()
    }
    await providerLogin(page, source, origin, 'unverified')
    await assertInvitationError('oauth_validation_failed')
    await providerLogin(page, source, origin)
    await page.getByRole('button', { name: 'Accept as ' + did, exact: true }).click()
    await verifyApp(page, ws, did, 'oauth')
    const sourceZone = zoneOf(ws.token)
    expect(await rpc(home, 'system_config', resolver, { did, source_zone: sourceZone }, ws.token)).toEqual({
      '@context': ['https://www.w3.org/ns/did/v1'],
      id: did,
    })
    expect((await page.context().cookies('https://' + home)).some((c) => c.name === 'buckyos_session_token')).toBe(
      false,
    )

    await page.goto(url)
    await providerLogin(page, source, origin, 'replacement')
    expect(new URL(page.url()).searchParams.get('did_error')).toBe('oauth_identity_conflict')
    const changedUrl = await issue(ws, { kind: 'directed' }, { source_id: source, email: 'changed@company.test' })
    await page.goto(changedUrl)
    await providerLogin(page, source, origin, 'renamed')
    await expect(
      page.getByRole('alert').filter({ hasText: 'This invitation is for a different identity' }),
    ).toBeVisible()
    await expect(page.getByRole('button', { name: 'Accept as ' + did, exact: true })).toHaveCount(0)
    const refusal = await page.evaluate(async (inviteId) => {
      const result = await fetch('/kapi/control-panel', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          method: 'auth.share.accept',
          params: { invite_id: inviteId, source: 'oauth' },
          sys: [Date.now()],
        }),
      })
      const reply = await result.json()
      return { error: reply.error, delivered: Boolean(reply.result?.callback_url) }
    }, new URL(changedUrl).pathname.split('/').pop()!)
    expect(refusal.error).toContain('different identity')
    expect(refusal.delivered).toBe(false)
    expect(new URL(page.url()).pathname).toBe(new URL(changedUrl).pathname)
    const open = await issue(ws, { kind: 'open' })
    await page.goto(open)
    await providerLogin(page, source, origin)
    await page.getByRole('button', { name: 'Accept as ' + did, exact: true }).click()
    await verifyApp(page, ws, did, 'oauth')
  })
}

test('a Zone without the provider refuses the external DID and accepts an invited home-account DID', async ({
  page,
}) => {
  const ws = await workspace(remote)
  const sourceZone = zoneOf(await owner(home))
  for (const { origin } of providers) {
    const did = fixtureDid(origin)
    expect(
      await rpc(remote, 'system_config', resolver, { did, source_zone: zoneOf(ws.token) }, ws.token),
    ).toBeNull()
    await expect(
      rpc(remote, 'system_config', resolver, { did, source_zone: sourceZone }, ws.token),
    ).rejects.toThrow()
  }
  const config = await rpc(remote, 'control-panel', 'auth.sso_config', {})
  expect(((config.oauth_sources ?? []) as Row[]).some((p) => p.id === 'google')).toBe(false)
  await expect(issue(ws, { kind: 'directed', did: 'did:web:alice.google.com' })).rejects.toThrow(/登录来源/)
  await expect(issue(ws, { kind: 'directed' }, { source_id: 'google', email: 'alice@gmail.com' })).rejects.toThrow(
    /登录来源/,
  )
  const username = 'dv' + randomBytes(5).toString('hex'),
    password = randomBytes(18).toString('base64url')
  const ht = await sudoForCreate()
  await rpc(
    home,
    'control-panel',
    'user.create',
    { user_id: username, user_type: 'user', password_hash: digest(password + username + '.buckyos') },
    ht,
  )
  const local = await rpc(
    home,
    'control-panel',
    'auth.current_identity',
    {},
    await login(home, username, password),
  )
  const did = String(local.did)
  const url = await issue(ws, { kind: 'directed', did })
  await fixedLocale(page)
  await page.goto(url)
  await page.getByTestId('sso-did-submit').click()
  await page.waitForURL((u) => u.hostname === home)
  await page.getByLabel('Username').fill(username)
  await page.getByLabel('Password').fill(password)
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  await expect(page.getByTestId('sso-federate-did')).toHaveText(did)
  await page.getByTestId('sso-federate-approve').click()
  await page.getByRole('button', { name: 'Accept as ' + did, exact: true }).click()
  await verifyApp(page, ws, did, 'did_federation')
})

test('a dedicated Google test account accepts an admin invite and federates as its home account', async ({
  browser,
}) => {
  test.skip(
    !googleState || !googleEmail,
    'provide a dedicated consented Google test session and its unused test email',
  )
  const ht = await owner(home)
  const config = await rpc(home, 'control-panel', 'auth.sso_config', {})
  expect(((config.oauth_sources ?? []) as Row[]).some((p) => p.id === 'google')).toBe(true)
  const username = 'dv' + randomBytes(5).toString('hex')
  const invitation = await rpc(
    home,
    'control-panel',
    'user.invite.create',
    { user_id: username, google_email: googleEmail, default_user_type: 'user' },
    ht,
  )
  const context = await browser.newContext({ storageState: googleState, ignoreHTTPSErrors: true })
  try {
    const page = await context.newPage()
    await fixedLocale(page)
    await page.goto(String(invitation.invite_url))
    await page.getByRole('button', { name: 'Continue with Google', exact: true }).click()
    await expect
      .poll(
        async () =>
          !!(await context.cookies('https://' + home)).find(
            (c) => c.domain === home && c.name === 'buckyos_session_token',
          ),
      )
      .toBe(true)
    const session = (await context.cookies('https://' + home)).find(
      (c) => c.domain === home && c.name === 'buckyos_session_token',
    )!.value
    const identity = await rpc(home, 'control-panel', 'auth.current_identity', {}, session)
    expect(identity).toMatchObject({
      kind: 'account',
      user_id: username,
      auth_method: 'oauth',
      oauth_source_id: 'google',
    })
    const did = String(identity.did)
    const ws = await workspace(remote)
    await page.goto(await issue(ws, { kind: 'directed', did }))
    await page.getByTestId('sso-did-submit').click()
    await expect(page.getByTestId('sso-federate-did')).toHaveText(did)
    await page.getByTestId('sso-federate-approve').click()
    await page.getByRole('button', { name: 'Accept as ' + did, exact: true }).click()
    await verifyApp(page, ws, did, 'did_federation')
  } finally {
    await context.close()
  }
})

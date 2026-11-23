import { createHash, randomUUID } from 'node:crypto'
import { expect, type Page, test } from '@playwright/test'

const zone = process.env.BUCKYOS_SHARE_ZONE || 'bob.web3.devtests.org'
const owner = process.env.BUCKYOS_SHARE_OWNER || 'bob'
const second = process.env.BUCKYOS_SHARE_SECOND_OWNER || 'sharetester'
const app = 'buckyos-systest.buckyos.bns.did'
const firstHost = process.env.BUCKYOS_SHARE_APP_HOST || `buckyos-systest.${zone}`
const secondHost = process.env.BUCKYOS_SHARE_SECOND_HOST || `buckyos-systest-${second}.${zone}`
const home = process.env.BUCKYOS_SHARE_HOME || 'charlie.me'
const visitor = process.env.BUCKYOS_SHARE_VISITOR || 'charlie'
const expectedDid = process.env.BUCKYOS_SHARE_VISITOR_DID || `did:bns:${visitor}`
const password = process.env.BUCKYOS_TEST_PASSWORD || 'bucky2025'
const rules = process.env.BUCKYOS_TEST_HOST_RULES || ''
const root = 'https://' + zone
const cookie = 'buckyos_session_token'
type Params = Record<string, unknown>
test.skip(!rules, 'Supply BUCKYOS_TEST_HOST_RULES with the actual Zone and App addresses')
test.use({
  screenshot: 'off',
  video: 'off',
  ignoreHTTPSErrors: true,
  launchOptions: {
    args: [
      '--ignore-certificate-errors',
      ...(process.env.BUCKYOS_SHARE_HTTP1 === '1' ? ['--disable-http2'] : []),
      '--host-resolver-rules=' + rules,
    ],
  },
  trace: 'off',
})

async function rpc(page: Page, service: string, method: string, params: Params, token?: string) {
  return page.evaluate(async ({ service, method, params, token }) => {
    const response = await fetch('/kapi/' + service, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ method, params, sys: token ? [Date.now(), token] : [Date.now()] }),
    })
    const value = await response.json()
    if (value.error || value.result?.ok === false) {
      throw Error(
        method + ': ' +
          String(
            value.error ||
              [value.result?.error?.code, value.result?.error?.detail, value.result?.error?.message].filter(Boolean)
                .join(' '),
          ).replace(/[A-Za-z0-9_.-]{60,}/g, '[…]'),
      )
    }
    return value.result
  }, { service, method, params, token })
}
async function session(page: Page, user: string, origin = root) {
  await page.goto(origin + '/login')
  const hash = (input: string) => createHash('sha256').update(input).digest('base64')
  const nonce = Date.now()
  const result = await rpc(page, 'control-panel', 'auth.login', {
    username: user,
    password: hash(hash(password + user + '.buckyos') + nonce),
    login_nonce: nonce,
    target: { kind: 'system', service_id: 'control-panel' },
  })
  if (!result.session_token) throw Error('Owner login unavailable')
  return String(result.session_token)
}
async function acceptNative(
  page: Page,
  url: string,
  targetHost: string,
  identity = { did: expectedDid, host: home, user: visitor },
  remember = false,
) {
  await page.goto(url)
  const field = page.getByTestId('sso-did-input')
  if (await field.isEnabled()) await field.fill(identity.did)
  else await expect(field).toHaveValue(identity.did)
  await page.getByTestId('sso-did-submit').click()
  await page.waitForURL((u) => u.hostname === identity.host)
  const consent = page.getByTestId('sso-federate-consent')
  const name = page.getByLabel('Username')
  await expect(name.or(consent)).toBeVisible()
  if (await name.isVisible()) {
    await name.fill(identity.user)
    await page.getByLabel('Password').fill(password)
    await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  }
  await expect(page.getByTestId('sso-federate-target')).toHaveText(targetHost)
  await expect(page.getByTestId('sso-federate-risk')).toContainText(targetHost)
  await page.getByTestId('sso-federate-remember').getByRole('checkbox').setChecked(remember)
  await page.getByTestId('sso-federate-approve').click()
  await page.getByRole('button', { name: 'Accept as ' + identity.did, exact: true }).click()
  await page.waitForURL((u) => u.host === targetHost && u.pathname !== '/sso_callback')
  const cookies = await page.context().cookies()
  expect(cookies.some((c) => c.name === cookie && c.domain === zone)).toBe(false)
  const targetSession = cookies.find((c) => c.name === cookie && c.domain === targetHost)
  if (!targetSession) throw Error('Missing target-host session')
  return targetSession.value
}

test('ordinary App keeps a directed document grant on its issuing installation', async ({ page, browser }) => {
  const issuing = await browser.newContext({ ignoreHTTPSErrors: true })
  const manager = await issuing.newPage()
  const credential = await session(manager, owner)
  const cp = (method: string, params: Params) => rpc(manager, 'control-panel', method, params, credential)
  const a = { app_instance_id: app + '@' + owner }, b = { app_instance_id: app + '@' + second }
  const initialA = await cp('apps.share.get', a), initialB = await cp('apps.share.get', b)
  const run = randomUUID(),
    resource = 'dv-' + run,
    deepLink = `https://${firstHost}/docs/${resource}?mode=edit#comments`
  try {
    await cp('apps.share.set', { ...a, level: 'directed' })
    await cp('apps.share.set', { ...b, level: 'disabled' })
    expect((await cp('apps.share.get', a)).level).toBe('directed')
    expect((await cp('apps.share.get', b)).level).toBe('disabled')
    const launch = await cp('auth.prepare_app_launch', { ...a, redirect_url: 'https://' + firstHost + '/' })
    await manager.goto(launch.callback_url)
    await manager.getByRole('link', { name: '打开可编辑的示例文档并邀请协作者' }).click()
    await expect(manager).toHaveURL('https://' + firstHost + '/docs/42?mode=edit#comments')
    await manager.goto(deepLink)
    await manager.getByLabel('Recipient DID or Google email').fill(expectedDid)
    await manager.getByLabel('Permission').selectOption('edit')
    await manager.getByRole('button', { name: 'Create invitation' }).click()
    await expect(manager.locator('#link')).toContainText('/sso/share/inv-')
    await issuing.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: 'https://' + firstHost })
    await manager.getByRole('button', { name: 'Copy link' }).click()
    const link = await manager.evaluate(() => navigator.clipboard.readText())
    expect(link).toBe(await manager.locator('#link').textContent())
    const secondContext = await browser.newContext({ ignoreHTTPSErrors: true })
    try {
      const secondPage = await secondContext.newPage()
      const secondCredential = await session(secondPage, second)
      const secondLaunch = await rpc(secondPage, 'control-panel', 'auth.prepare_app_launch', {
        ...b,
        redirect_url: 'https://' + secondHost + '/docs/' + resource,
      }, secondCredential)
      await secondPage.goto(secondLaunch.callback_url)
      await expect(secondPage.locator('h1')).toHaveText('Document ' + resource)
      const forbidden = await secondPage.evaluate(
        async ({ resource, did }) =>
          (await fetch('/sdk/appservice/documents/' + resource + '/invitations', {
            method: 'POST',
            headers: { 'content-type': 'application/json' },
            body: JSON.stringify({
              request_id: crypto.randomUUID(),
              action: 'read',
              audience: { kind: 'directed', did },
            }),
          })).status,
        { resource, did: expectedDid },
      )
      expect(forbidden).toBe(403)
    } finally {
      await secondContext.close()
    }
    const targetSession = await acceptNative(page, link, firstHost)
    await expect(page).toHaveURL(deepLink)
    await expect(page.locator('h1')).toHaveText('Document ' + resource)
    await page.getByLabel('Document text').fill(run)
    await page.getByRole('button', { name: 'Save', exact: true }).click()
    await expect(page.getByRole('status')).toContainText('Saved')
    await manager.reload()
    await expect(manager.getByLabel('Document text')).toHaveValue(run)
    expect(
      await page.evaluate(async () => (await fetch('/sdk/appservice/documents/other-private-document')).status),
    ).toBe(404)
    const description = await cp('auth.share.describe', { invite_id: new URL(link).pathname.split('/').pop() })
    const id = description.resource_id
    const close = (did?: string) =>
      manager.evaluate(async ({ id, resource, did }) => {
        const res = await fetch('/sdk/appservice/documents/' + resource + '/invitations', {
          method: 'DELETE',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ request_id: id, did }),
        })
        return res.status
      }, { id, resource, did })
    expect(await close()).toBe(200)
    expect(await page.evaluate(async () => (await fetch('/sso_refresh', { method: 'POST' })).status)).toBe(200)
    expect(
      await page.evaluate(
        async (resource) => (await fetch('/sdk/appservice/documents/' + resource)).status,
        resource,
      ),
    ).toBe(200)
    await page.goto('https://' + secondHost + '/docs/' + resource)
    const status = await page.evaluate(
      async ({ resource, token }) =>
        (await fetch('/sdk/appservice/documents/' + resource, { headers: { Authorization: 'Bearer ' + token } }))
          .status,
      { resource, token: targetSession },
    )
    expect([401, 403, 404]).toContain(status)
    expect(await close(expectedDid)).toBe(200)
    await page.goto(deepLink)
    const removed = await page.evaluate(
      async (resource) => (await fetch('/sdk/appservice/documents/' + resource)).status,
      resource,
    )
    expect([401, 403, 404]).toContain(removed)
    const refresh = await page.evaluate(async () => {
      const response = await fetch('/sso_refresh', { method: 'POST' })
      const data = await response.json()
      return {
        status: response.status,
        error: String(data.error || '').replace(/[a-z0-9_.=-]{55,}/ig, '[…]'),
        issued: !!data.session_token,
      }
    })
    expect(refresh.issued).toBe(false)
    expect([400, 401, 403]).toContain(refresh.status)
    expect(refresh.error).toMatch(/resource.grant.missing|revoked|invalid.token|denied|no permission/i)
    expect((await page.context().cookies('https://' + firstHost)).some((c) => c.name === cookie)).toBe(false)
  } finally {
    await manager.goto(root + '/login')
    await cp('apps.share.set', { ...a, level: initialA.level })
    await cp('apps.share.set', { ...b, level: initialB.level })
    await issuing.close()
  }
})

test('Message Hub delivers a visitor session to its own host and admits the accepted group', async ({ page, browser }) => {
  const managing = await browser.newContext({ ignoreHTTPSErrors: true })
  const manager = await managing.newPage()
  const credential = await session(manager, owner)
  const msg = (method: string, params: Params) => rpc(manager, 'msg-center', method, params, credential)
  const run = randomUUID()
  const made = await msg('group.create', {
    idempotency_key: run,
    configuration: {
      profile: { name: 'Share DV ' + run },
      membership: {
        guest_entry: { session_template: 'tickets', max_open_per_guest: 1, auth_methods: { did_login: true } },
      },
      session_templates: { tickets: { membership: 'explicit', rules: { allow_guests: true } } },
    },
  })
  const group = made.group_did
  try {
    const returned = await msg('group.issue_share', {
      group_did: group,
      request_id: run,
      audience: { kind: 'directed', did: expectedDid },
    })
    expect(returned.url).toContain('/sso/share/inv-')
    const host = 'messagehub.' + zone
    const target = await acceptNative(page, returned.url, host)
    expect(new URL(page.url()).searchParams.get('group')).toBe(group)
    const ticket = await rpc(page, 'msg-center', 'group.submit_guest_request', {
      group_did: group,
      request_id: run,
    }, target)
    expect(ticket.session_id).toBeTruthy()
    const members = await msg('group.list_members', { group_did: group })
    expect(members.items.some((member: Params) => member.member_did === expectedDid)).toBe(false)
    const sessions = await rpc(page, 'msg-center', 'group.list_sessions', { group_did: group }, target)
    expect(sessions.items.some((item: Params) => item.session_id === ticket.session_id)).toBe(true)
  } finally {
    await msg('group.delete', { group_did: group })
    await managing.close()
  }
})

test('two native recipients independently accept the same open App invitation', async ({ browser }) => {
  const issuer = await browser.newContext({ ignoreHTTPSErrors: true })
  const manager = await issuer.newPage()
  const credential = await session(manager, owner)
  const cp = (method: string, params: Params) => rpc(manager, 'control-panel', method, params, credential)
  const install = { app_instance_id: app + '@' + owner }
  const initial = await cp('apps.share.get', install)
  const resource = 'open-' + randomUUID(),
    path = '/docs/' + resource + '?mode=edit#comments',
    deep = 'https://' + firstHost + path
  try {
    await cp('apps.share.set', { ...install, level: 'open' })
    const launch = await cp('auth.prepare_app_launch', { ...install, redirect_url: deep })
    await manager.goto(launch.callback_url)
    await manager.getByLabel('Audience').selectOption('open')
    await manager.getByLabel('Permission').selectOption('edit')
    await manager.getByRole('button', { name: 'Create invitation' }).click()
    await expect(manager.locator('#link')).toContainText('/sso/share/inv-')
    const link = (await manager.locator('#link').textContent())!
    const identities = [{ did: expectedDid, host: home, user: visitor }, {
      did: process.env.BUCKYOS_SHARE_OTHER_DID || 'did:bns:alice',
      host: process.env.BUCKYOS_SHARE_OTHER_ZONE || 'alice.web3.devtests.org',
      user: process.env.BUCKYOS_SHARE_OTHER_USER || 'alice',
    }]
    for (const identity of identities) {
      const visiting = await browser.newContext({ ignoreHTTPSErrors: true })
      try {
        const tab = await visiting.newPage()
        await acceptNative(tab, link, firstHost, identity)
        await expect(tab.locator('h1')).toHaveText('Document ' + resource)
        await tab.getByLabel('Document text').fill(identity.did)
        await tab.getByRole('button', { name: 'Save', exact: true }).click()
        await expect(tab.getByRole('status')).toContainText('Saved')
        const document = await manager.evaluate(
          async (id) => (await fetch('/sdk/appservice/documents/' + id)).json(),
          resource,
        )
        expect(document.body).toBe(identity.did)
      } finally {
        await visiting.close()
      }
    }
  } finally {
    await manager.goto(root + '/login')
    await cp('apps.share.set', { ...install, level: initial.level })
    await issuer.close()
  }
})

test('remembered consent for one installation still requires disclosure on the other', async ({ page, browser }) => {
  const contexts = await Promise.all([
    browser.newContext({ ignoreHTTPSErrors: true }),
    browser.newContext({ ignoreHTTPSErrors: true }),
    browser.newContext({ ignoreHTTPSErrors: true }),
  ])
  const [firstPage, secondPage, homePage] = await Promise.all(contexts.map((context) => context.newPage()))
  const [firstToken, secondToken, homeToken] = await Promise.all([
    session(firstPage, owner),
    session(secondPage, second),
    session(homePage, visitor, 'https://' + home),
  ])
  const homeRpc = (method: string, params: Params) => rpc(homePage, 'control-panel', method, params, homeToken)
  const original = await homeRpc('auth.federation.consents', {})
  const before = new Set<string>(original.consents.map((c: Params) => String(c.id)))
  const installs = [{ user: owner, tab: firstPage, token: firstToken, host: firstHost }, {
    user: second,
    tab: secondPage,
    token: secondToken,
    host: secondHost,
  }]
  const policies = await Promise.all(
    installs.map(({ user, tab, token }) =>
      rpc(tab, 'control-panel', 'apps.share.get', { app_instance_id: app + '@' + user }, token)
    ),
  )
  try {
    const links: string[][] = []
    for (const { user, tab, token, host } of installs) {
      const appInstanceId = app + '@' + user
      await rpc(
        tab,
        'control-panel',
        'apps.share.set',
        { app_instance_id: appInstanceId, level: 'directed' },
        token,
      )
      const resource = 'consent-' + randomUUID()
      const launch = await rpc(tab, 'control-panel', 'auth.prepare_app_launch', {
        app_instance_id: appInstanceId,
        redirect_url: 'https://' + host + '/docs/' + resource,
      }, token)
      await tab.goto(launch.callback_url)
      await expect(tab.locator('h1')).toHaveText('Document ' + resource)
      const urls = await tab.evaluate(async ({ resource, did }) => {
        const list: string[] = []
        for (let i = 0; i < 2; i++) {
          const response = await fetch('/sdk/appservice/documents/' + resource + '/invitations', {
            method: 'POST',
            headers: { 'content-type': 'application/json' },
            body: JSON.stringify({
              request_id: crypto.randomUUID(),
              action: 'edit',
              audience: { kind: 'directed', did },
            }),
          })
          const result = await response.json()
          if (!response.ok || !result.url) throw Error('Invitation unavailable')
          list.push(result.url)
        }
        return list
      }, { resource, did: expectedDid })
      links.push(urls)
    }
    const signed = { did: expectedDid, host: home, user: visitor }
    await acceptNative(page, links[0][0], firstHost, signed, true)
    await page.goto(links[0][1])
    await page.getByTestId('sso-did-submit').click()
    await page.getByRole('button', { name: 'Accept as ' + expectedDid, exact: true }).click()
    await page.waitForURL((u) => u.host === firstHost && u.pathname.startsWith('/docs/'))
    await acceptNative(page, links[1][0], secondHost)
  } finally {
    const current = await homeRpc('auth.federation.consents', {})
    for (const consent of current.consents) {
      if (!before.has(consent.id) && consent.target === 'app:' + app + '@' + owner) {
        await homeRpc('auth.federation.forget', { id: consent.id })
      }
    }
    for (let i = 0; i < installs.length; i++) {
      const { tab, user, token } = installs[i]
      await tab.goto(root + '/login')
      await rpc(tab, 'control-panel', 'apps.share.set', {
        app_instance_id: app + '@' + user,
        level: policies[i].level,
      }, token)
    }
    await Promise.all(contexts.map((c) => c.close()))
  }
})

test('downgrade rejects an unused open invitation and stopping the installation ends resource access', async ({ page, browser }) => {
  test.setTimeout(360_000)
  const issuer = await browser.newContext({ ignoreHTTPSErrors: true })
  const managing = await issuer.newPage(), appPage = await issuer.newPage()
  const credential = await session(managing, second)
  const install = { app_instance_id: app + '@' + second }
  const cp = (method: string, params: Params) => rpc(managing, 'control-panel', method, params, credential)
  const policy = await cp('apps.share.get', install)
  const status = () => cp('apps.status', install)
  const initial = await status()
  expect(initial.desired_spec.state).toBe('running')
  expect(initial.ready_instance_count).toBeGreaterThan(0)
  const doc = 'lifecycle-' + randomUUID()
  let stopRequested = false
  try {
    await cp('apps.share.set', { ...install, level: 'open' })
    const launch = await cp('auth.prepare_app_launch', {
      ...install,
      redirect_url: 'https://' + secondHost + '/docs/' + doc,
    })
    await appPage.goto(launch.callback_url)
    await expect(appPage.locator('h1')).toHaveText('Document ' + doc)
    const issue = async (audience: Params) =>
      appPage.evaluate(async ({ doc, audience }) => {
        const response = await fetch('/sdk/appservice/documents/' + doc + '/invitations', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ request_id: crypto.randomUUID(), audience, action: 'read' }),
        })
        if (!response.ok) throw Error('Could not issue invitation: ' + response.status)
        return String((await response.json()).url)
      }, { doc, audience })
    const open = await issue({ kind: 'open' })
    const record = { invite_id: new URL(open).pathname.split('/').pop() }
    const before = await cp('auth.share.describe', record)
    expect(before.audience).toEqual({ kind: 'open' })
    expect(before.unavailable_reason).toBeUndefined()
    await cp('apps.share.set', { ...install, level: 'directed' })
    await expect(cp('auth.share.accept', { ...record, source: 'session' })).rejects.toThrow(
      /installation does not allow this invitation audience/,
    )
    const { unavailable_reason, ...retained } = await cp('auth.share.describe', record)
    expect(unavailable_reason).toMatch(/installation does not allow this invitation audience/)
    expect(retained).toEqual(before)

    const directed = await issue({ kind: 'directed', did: expectedDid })
    const token = await acceptNative(page, directed, secondHost)
    const verify = () =>
      rpc(managing, 'verify-hub', 'verify_token', {
        session_token: token,
        expected_target: { kind: 'app', ...install },
      })
    expect(await verify()).toBe(true)
    expect(await page.evaluate(async (id) => (await fetch('/sdk/appservice/documents/' + id)).status, doc)).toBe(200)
    const stop = await cp('apps.stop', { ...install, idempotency_key: randomUUID() })
    stopRequested = true
    expect(stop.app_instance_id).toBe(install.app_instance_id)
    await expect.poll(async () => (await status()).desired_spec.state, { timeout: 45_000 }).toBe('stopped')
    await expect(verify()).rejects.toThrow(/app_runtime_unavailable|entry_unavailable|family.*(invalid|ended)|revoked/i)
    const renewal = await page.evaluate(async () => {
      const response = await fetch('/sso_refresh', { method: 'POST' })
      const result = await response.json().catch(() => ({}))
      return { status: response.status, issued: !!result.session_token }
    })
    expect(renewal.status).toBeGreaterThanOrEqual(400)
    expect(renewal.issued).toBe(false)
    await expect.poll(async () => (await status()).ready_instance_count, { timeout: 90_000 }).toBe(0)
    expect((await cp('apps.share.set', { ...install, level: 'disabled' })).level).toBe('disabled')
    expect((await cp('apps.share.get', install)).level).toBe('disabled')
  } finally {
    try {
      if (stopRequested) {
        const startKey = randomUUID()
        await expect.poll(async () => {
          try {
            return !!(await cp('apps.start', { ...install, idempotency_key: startKey })).task_id
          } catch (error) {
            if (/busy|conflict|another|mutation|in progress/i.test(String(error))) return false
            throw error
          }
        }, { timeout: 120_000 }).toBe(true)
        await expect.poll(async () => {
          const state = await status()
          return state.desired_spec.state === 'running' && state.ready_instance_count > 0
        }, { timeout: 120_000 }).toBe(true)
      }
      await cp('apps.share.set', { ...install, level: policy.level })
    } finally {
      await issuer.close()
    }
  }
})

test('an open Workspace invitation admits an anonymous reader until the owner closes it', async ({ page, browser }) => {
  const managerContext = await browser.newContext({ ignoreHTTPSErrors: true })
  const closedContext = await browser.newContext({ ignoreHTTPSErrors: true })
  const accountContext = await browser.newContext({ ignoreHTTPSErrors: true })
  const manager = await managerContext.newPage()
  const token = await session(manager, owner)
  const accountId = 'sharetest-' + randomUUID().slice(0, 12)
  const accountPassword = randomUUID()
  let registered = false
  const workspace = (method: string, params: Params) => rpc(manager, 'aiworkspace', method, params, token)
  const title = 'guest-' + randomUUID()
  const created = await workspace('ws.create', { title })
  const at = { workspace_id: created.workspace_id }
  const workspaceHost = 'aiws-' + String(created.workspace_id).replace(/^ws_/, '') + '.' + zone
  const field = { field_id: 'proof', name: title, type: 'text' }
  const anonymous = async (tab: Page) => {
    const credential = await tab.evaluate(() => localStorage.getItem('aiworkspace.guest'))
    if (!credential?.startsWith('gt_')) throw Error('Missing workspace guest identity')
    return credential
  }
  const assertNoZoneSession = async (tab: Page) => {
    const cookies = await tab.context().cookies()
    expect(cookies.some((c) => c.name === cookie || c.name === 'buckyos_refresh_token')).toBe(false)
  }
  try {
    const written = await workspace('doc.commit', {
      ...at,
      protocol_version: '1.0',
      epoch: created.epoch,
      idempotency_key: randomUUID(),
      session_id: 'guest-test',
      operations: [{
        op: 'entity.create',
        entity_id: 'schedule',
        type_id: 'buckyos.table-source',
        parent_id: 'data',
        order_key: 'a',
        name: title,
        payload: { fields: [field] },
      }],
    })
    expect(written.status).toBe('accepted')
    await workspace('acl.set_group', { ...at, group_id: 'listeners', title: 'Listeners' })
    await workspace('acl.set', { ...at, changes: [{ subject: '@listeners', resource: '/', allow: ['read'] }] })
    const issued = await workspace('ws.issue_share', {
      ...at,
      group_id: 'listeners',
      request_id: randomUUID(),
      methods: { guest: true, self_register: true },
      audience: { kind: 'open' },
      return_path: '/' + title,
    })
    const entry = issued.entry
    expect(new URL(entry.share_url).pathname).toMatch(/^\/sso\/share\/inv-/)
    await page.goto(entry.share_url)
    const guestChoice = page.getByRole('link', { name: /continue as (a )?guest/i })
    await expect(page.getByRole('button', { name: 'Create an account here' })).toBeVisible()
    await guestChoice.click()
    await page.getByTestId('sso-entry-method-guest').click()
    await page.waitForURL((url) => url.hostname === workspaceHost)
    await expect(page.getByRole('toolbar', { name: 'Main toolbar' })).toContainText(title)
    const guest = await anonymous(page)
    const info = await rpc(page, 'aiworkspace', 'ws.get_info', at, guest)
    expect(info.principal).toMatch(/^guest:/)
    expect(info.capabilities).toEqual(['read'])
    const document = await rpc(page, 'aiworkspace', 'doc.read', { ...at, entity_id: 'schedule' }, guest)
    expect(document.content.fields).toMatchObject([field])
    await assertNoZoneSession(page)

    const applicant = await accountContext.newPage()
    await applicant.goto(entry.share_url)
    await applicant.getByRole('button', { name: 'Create an account here' }).click()
    await applicant.getByLabel('User name', { exact: true }).fill(accountId)
    await applicant.getByLabel('Password', { exact: true }).fill(accountPassword)
    await applicant.getByLabel('Confirm password', { exact: true }).fill(accountPassword)
    const createdAccount = applicant.waitForResponse((response) =>
      response.url().endsWith('/kapi/control-panel') &&
      response.request().postDataJSON()?.method === 'auth.register'
    )
    await applicant.getByTestId('sso-register-submit').click()
    const result = (await (await createdAccount).json()).result
    registered = result?.status === 'created'
    expect(result?.status).toBe('created')
    const accept = applicant.getByRole('button', { name: /^Accept as did:/ })
    await expect(accept).toBeVisible()
    expect(applicant.url()).toBe(entry.share_url)
    const did = (await accept.textContent())!.replace(/^Accept as /, '').trim()
    const rootCookie = (await accountContext.cookies(root)).find((c) => c.name === cookie && c.domain === zone)
    if (!rootCookie) throw Error('Missing registered account session on its own Zone')
    const accountIdentity = await rpc(applicant, 'control-panel', 'auth.current_identity', {}, rootCookie.value)
    expect(accountIdentity).toMatchObject({ kind: 'account', user_id: accountId, did })
    await accept.click()
    await applicant.waitForURL((url) => url.hostname === workspaceHost && url.pathname === '/' + title)
    const appSession = (await accountContext.cookies('https://' + workspaceHost)).find((c) =>
      c.name === cookie && c.domain === workspaceHost
    )
    if (!appSession) throw Error('Missing registered reader workspace session')
    const claims = JSON.parse(Buffer.from(appSession.value.split('.')[1], 'base64url').toString())
    expect(claims.sub).toBe(accountId)
    const directory = await rpc(manager, 'control-panel', 'user.invite.directory', {}, token)
    const record = directory.users.find((row: Params) => row.did === did)
    const inviteId = new URL(entry.share_url).pathname.split('/').pop()
    expect(claims.resource_share).toBe(inviteId)
    expect(claims.resource_share_did).toBe(did)
    expect(record.sources.find((source: Params) => source.invite_id === inviteId)?.accepted_at).toBeGreaterThan(0)
    const accountInfo = await rpc(applicant, 'aiworkspace', 'ws.get_info', at, appSession.value)
    expect(accountInfo.principal).toBe(accountId)
    expect(accountInfo.capabilities).toEqual(['read'])
    const accountDocument = await rpc(
      applicant,
      'aiworkspace',
      'doc.read',
      { ...at, entity_id: 'schedule' },
      appSession.value,
    )
    expect(accountDocument.content.fields).toMatchObject([field])

    await workspace('ws.close_entry', { ...at, entry_id: entry.entry_id })
    const closed = await closedContext.newPage()
    await closed.goto(entry.share_url)
    await expect(closed.getByTestId('sso-share-page').getByRole('alert')).toBeVisible()
    await expect(closed.getByRole('link', { name: /continue as (a )?guest/i })).toHaveCount(0)
    await expect(closed.getByRole('button', { name: 'Create an account here' })).toHaveCount(0)
    const entryUrl = new URL('/sso/entry', root)
    entryUrl.search = new URLSearchParams({ service: 'aiworkspace', entry: entry.entry_id }).toString()
    await closed.goto(entryUrl.href)
    await expect(closed.getByTestId('sso-entry-unavailable')).toBeVisible()
    await expect(closed.getByTestId('sso-entry-method-guest')).toHaveCount(0)
    const direct = new URL('/' + title, 'https://' + workspaceHost)
    direct.searchParams.set('entry', entry.entry_id)
    await closed.goto(direct.href)
    await closed.waitForURL((url) => url.hostname === workspaceHost && !url.searchParams.has('entry'))
    const fresh = await anonymous(closed)
    expect(fresh === guest).toBe(false)
    await expect(rpc(closed, 'aiworkspace', 'ws.join_entry', { entry_id: entry.entry_id }, fresh)).rejects.toThrow(
      /closed|unavailable/i,
    )
    await expect(rpc(closed, 'aiworkspace', 'doc.read', { ...at, entity_id: 'schedule' }, fresh)).rejects.toThrow(
      /NOT_FOUND|PERMISSION_DENIED/i,
    )
    await assertNoZoneSession(closed)
  } finally {
    try {
      await workspace('ws.delete', at)
      if (registered) {
        const hash = (input: string) => createHash('sha256').update(input).digest('base64')
        const nonce = Date.now()
        const sudo = await rpc(manager, 'verify-hub', 'sudo_by_password', {
          username: owner,
          password: hash(hash(password + owner + '.buckyos') + nonce),
          login_nonce: nonce,
          target: { kind: 'system', service_id: 'control-panel' },
          aud: 'system-config',
        })
        await rpc(manager, 'control-panel', 'user.delete', { user_id: accountId }, sudo.session_token)
      }
    } finally {
      await Promise.all([managerContext.close(), closedContext.close(), accountContext.close()])
    }
  }
})

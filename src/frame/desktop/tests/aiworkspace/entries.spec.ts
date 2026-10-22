/* Hybrid SSO (SSO-13, PRE-03): entry links in the share panel — ways in as checkboxes, the link to the Zone's entry
 * page, ways the Zone switched off marked (the Control Panel's `auth.entry.describe`, answered by the mock Control
 * Panel of the Desktop shell here), guests let in while the entry is open — and what a restricted session (a
 * Limited account) sees: no workspace creation, and the category of a refusal instead of a sign-in prompt. */

import { expect, openCanvas, test } from './fixtures'

const ALICE = 'tok-alice'
const LIMITED = 'tok-lim'
/** The mock Control Panel keeps its state here (src/api/control_panel_mock.ts) and re-reads it on every call. */
const MOCK_CP_KEY = 'buckyos.mock.control-panel.v1'

test('UI-SSO1 entry links: create with chosen ways in, Zone-closed ways marked, anyone in while open, closed again', async ({ page, api, browser }) => {
  const ws = await api.demo(ALICE, 'quarterly', `sso1 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-share').click()
  const entries = page.getByTestId('aiws-share-entries')
  await expect(entries.getByTestId('aiws-share-entry-none')).toBeVisible()
  // no way in, no entry
  for (const method of ['guest', 'password']) await entries.getByTestId(`aiws-share-entry-${method}`).uncheck()
  await expect(entries.getByTestId('aiws-share-entry-create')).toBeDisabled()
  await entries.getByTestId('aiws-share-entry-guest').check()
  await entries.getByTestId('aiws-share-entry-password').check()
  await entries.getByTestId('aiws-share-entry-did_login').check()
  await entries.getByTestId('aiws-share-entry-preset').selectOption('commenter')
  await entries.getByTestId('aiws-share-entry-create').click()
  await expect(entries.getByTestId('aiws-share-entry')).toHaveCount(1)
  const listed = await api.rpc(ALICE, 'ws.list_entries', { workspace_id: ws.workspace_id })
  expect(listed.entries).toHaveLength(1)
  const entry = listed.entries[0]
  expect(entry.methods).toEqual({ guest: true, password: true, self_register: false, did_login: true })
  expect(entry.capabilities).toEqual(['read', 'comment'])
  expect(entry.entry_id).toMatch(new RegExp(`^${ws.workspace_id}~se_[a-z2-7]{26}$`))
  const link = new URL(await entries.getByTestId('aiws-share-entry-link').inputValue())
  expect(link.pathname).toBe('/sso/entry')
  expect(Object.fromEntries(link.searchParams)).toEqual({ service: 'aiworkspace', entry: entry.entry_id })
  // the public description the entry page starts from (no session)
  const described = await (await fetch(api.base, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ method: 'entry.describe', params: { entry_id: entry.entry_id }, sys: [1] }) })).json()
  expect(described.result.state).toBe('active')
  expect(described.result.resource).toMatchObject({ kind: 'workspace', owner_name: 'alice', path: `/workspace/${ws.workspace_id}` })

  // the Zone turned DID sign-in off: the panel says so on the entry (the Control Panel intersects the methods)
  await page.evaluate(({ key, id, methods }) => {
    const state = JSON.parse(window.localStorage.getItem(key) ?? '{}')
    state.sso = { ...(state.sso ?? {}), settings: { self_register_enabled: true, did_login_enabled: false }, entries: [{ service: 'aiworkspace', entry_id: id, state: 'active', methods }] }
    window.localStorage.setItem(key, JSON.stringify(state))
  }, { key: MOCK_CP_KEY, id: entry.entry_id, methods: entry.methods })
  await page.keyboard.press('Escape')
  await page.getByTestId('aiws-share').click()
  const row = page.getByTestId('aiws-share-entry')
  await expect(row.getByTestId('aiws-share-entry-method-did_login')).toHaveAttribute('data-zone-disabled', 'true')
  await expect(row.getByTestId('aiws-share-entry-method-did_login')).toContainText('已被 Zone 设置关闭')
  await expect(row.getByTestId('aiws-share-entry-method-guest')).not.toHaveAttribute('data-zone-disabled', 'true')

  // an entry open to guests lets anyone in, a Limited account included (there is no public grant)
  expect((await api.rpc(LIMITED, 'ws.get_info', { workspace_id: ws.workspace_id })).capabilities).toEqual(['read', 'comment'])
  // a guest with the link gets the entry's access
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const guest = await context.newPage()
  await guest.addInitScript(() => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ guest: true })))
  await guest.goto(`/workspace/${ws.workspace_id}`)
  await expect(guest.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', ws.workspace_id, { timeout: 30_000 })
  await expect(guest.getByTestId('aiws-principal')).toHaveAttribute('data-guest', 'true')
  await context.close()

  // closing it in the panel: guests are out at their next request
  await row.getByTestId('aiws-share-entry-close').click()
  await expect(page.getByTestId('aiws-share-entry')).toHaveCount(0)
  expect((await api.rpc(ALICE, 'ws.list_entries', { workspace_id: ws.workspace_id })).entries[0].state).toBe('closed')
  const token = 'gt_abcdefghijklmnopqrstuvwxyz'
  const info = await api.rpc(token, 'ws.get_info', { workspace_id: ws.workspace_id })
  expect(info.error.denial.reason).toBe('resource_grant_missing')
})

test('UI-SSO2 a restricted session owns nothing and is told the category of a refusal', async ({ page, api }) => {
  const shared = await api.demo(ALICE, 'quarterly', `sso2 shared ${Date.now()}`)
  const hidden = await api.demo(ALICE, 'quarterly', `sso2 hidden ${Date.now()}`)
  await api.rpc(ALICE, 'ws.grant', { workspace_id: shared.workspace_id, subject: '*', capabilities: ['read'] })
  await api.rpc(ALICE, 'ws.grant', { workspace_id: shared.workspace_id, subject: 'acme-c1', capabilities: ['read', 'comment'] })
  // the service refuses creation with its reason; `*` does not reach the Limited account
  const created = await api.rpc(LIMITED, 'ws.create', { title: 'x' })
  expect([created.error.code, created.error.denial.reason]).toEqual(['PERMISSION_DENIED', 'session_kind_not_allowed'])
  expect((await api.rpc(LIMITED, 'ws.get_info', { workspace_id: shared.workspace_id })).capabilities).toEqual(['read', 'comment'])

  await page.addInitScript((value) => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ token: value, restricted: true })), LIMITED)
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-aiworkspace').click()
  await expect(page.getByTestId('aiws-list')).toBeVisible({ timeout: 30_000 })
  await expect(page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${shared.workspace_id}"]`)).toBeVisible()
  await expect(page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${hidden.workspace_id}"]`)).toHaveCount(0)
  await expect(page.getByTestId('aiws-new')).toHaveCount(0)
  await expect(page.getByTestId('aiws-new-template')).toHaveCount(0)

  // a link to a workspace not shared with it: the refusal's category for a visitor, no sign-in redirect
  await page.goto(`/workspace/${hidden.workspace_id}`)
  await expect(page.getByTestId('aiws-open-error')).toContainText('它不存在，或你没有访问权限，请联系分享者', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-open-error')).not.toContainText('分享面板')
  expect(new URL(page.url()).pathname).not.toBe('/login')
  // the download routes carry the same denial, and the transport hands it on
  const routeReason = await page.evaluate(async (id) => {
    const source = '/src/app/aiworkspace/api/transport.ts'
    const transport = await import(/* @vite-ignore */ source) as {
      resolveTransport: () => { ok: boolean; transport?: { download: (path: string) => Promise<Blob> } }
      TransportError: new (...args: never[]) => Error & { denial: { reason: string } | null }
    }
    const available = transport.resolveTransport()
    try {
      await available.transport?.download(`asset/${id}/missing`)
      return 'downloaded'
    } catch (error) {
      return error instanceof transport.TransportError ? error.denial?.reason ?? 'no denial' : String(error)
    }
  }, hidden.workspace_id)
  expect(routeReason).toBe('resource_grant_missing')
})

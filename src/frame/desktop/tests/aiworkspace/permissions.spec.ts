/* Phase two §13.1 UI14: preset groups, workspace and canvas permissions through the UI, union
 * semantics, and no leaks of names, referrers or dependency details to a restricted principal. */

import { expect, openWorkspace, test } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'

test('UI14 presets, canvas permissions and what a restricted principal can see', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `perm ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-left-permissions').click()
  await expect(page.getByTestId('aiws-permissions')).toBeVisible()
  await expect(page.locator('[data-testid="aiws-grant"][data-subject="alice"]').getByTestId('aiws-grant-preset')).toHaveText('管理者')
  // bob becomes a reader of the workspace, and an editor of the detail canvas only (two grants: Surface + content folder)
  await page.getByTestId('aiws-grant-subject').fill('bob')
  await page.getByTestId('aiws-grant-preset-select').selectOption('reader')
  await page.getByTestId('aiws-grant-submit').click()
  await expect(page.getByTestId('aiws-permissions-message')).toContainText('已授予 bob')
  await page.getByTestId('aiws-grant-subject').fill('bob')
  await page.getByTestId('aiws-grant-preset-select').selectOption('editor')
  await page.getByTestId('aiws-grant-scope').selectOption('sf-detail')
  await page.getByTestId('aiws-grant-submit').click()
  await expect(page.getByTestId('aiws-permissions-message')).toContainText('（画布）')
  await expect(page.locator('[data-testid="aiws-grant"][data-subject="bob"][data-scope=""]').getByTestId('aiws-grant-preset')).toHaveText('阅读者')
  await expect(page.locator('[data-testid="aiws-grant"][data-subject="bob"][data-scope="sf-detail"]').getByTestId('aiws-grant-preset')).toHaveText('编辑者')
  // the entries behind the panel (acl.query by: all): the workspace, the canvas and its content folder
  type Entry = { subject: string; resource: { entity_id: string | null } }
  const entries = async (token: string) => api.rpc(token, 'acl.query', { workspace_id: ws.workspace_id, by: 'all' }) as Promise<{ entries: Entry[]; complete: boolean }>
  const bobs = (await entries(ALICE)).entries.filter((e) => e.subject === 'bob').map((e) => e.resource.entity_id).sort()
  expect(bobs).toEqual([null, 'sf-detail', 'sf-detail-content'].sort())
  // bob: edits on the detail canvas are accepted (union), first-level data edits are refused
  const bobGrants = await entries(BOB)
  expect(bobGrants.complete).toBe(false)
  expect(bobGrants.entries.every((e) => e.subject === 'bob')).toBe(true)
  expect(JSON.stringify(bobGrants)).not.toContain('"alice"')
  const place = await api.commit(BOB, ws, [{ op: 'tree.place', entity_id: 'blk-chart-2', placement: { x: 1, y: 2, w: 420, h: 300 } }])
  expect(place.status).toBe('accepted')
  const note = await api.commit(BOB, ws, [{ op: 'entity.create', entity_id: 'bob-note', type_id: 'buckyos.annotation', parent_id: 'sf-detail-content', order_key: 'zz', payload: { kind: 'note', body: 'bob 的便签' } }])
  expect(note.status).toBe('accepted')
  const rev = (await api.cell(BOB, ws.workspace_id, 'sales', 's-1', 'cost')).rev
  const edit = await api.commit(BOB, ws, [{ op: 'table.set_values', source_id: 'sales', values: [{ record_id: 's-1', field_id: 'cost', value: 1, expect: { rev } }] }])
  expect(edit.status).toBe('rejected')
  expect(edit.code).toBe('PERMISSION_DENIED')
  const moveMain = await api.commit(BOB, ws, [{ op: 'tree.place', entity_id: 'blk-kpi', placement: { x: 1, y: 2, w: 420, h: 130 } }])
  expect(moveMain.status).toBe('rejected')
  // revoking the canvas permission removes both rows; the workspace grant still makes bob a reader
  await page.locator('[data-testid="aiws-grant"][data-subject="bob"][data-scope="sf-detail"]').getByTestId('aiws-revoke').click()
  await expect(page.locator('[data-testid="aiws-grant"][data-subject="bob"][data-scope="sf-detail"]')).toHaveCount(0)
  expect((await entries(ALICE)).entries.filter((e) => e.subject === 'bob')).toHaveLength(1)
  expect((await api.rpc(BOB, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'sales' })).ok).toBe(true)
  // a principal reading only a subtree learns nothing about referrers or dependencies elsewhere
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob' }] })
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: 'sf-detail-content', allow: ['read'] }] })
  const relations = await api.rpc(BOB, 'doc.relations', { workspace_id: ws.workspace_id, entity_id: 'bob-note' })
  expect(relations.ok).toBe(true)
  expect(relations.hidden_incoming).toBe(false)
  const salesRel = await api.rpc(BOB, 'doc.relations', { workspace_id: ws.workspace_id, entity_id: 'sales' })
  expect(salesRel.ok).toBe(false)
  expect(salesRel.error.code).toBe('NOT_FOUND')
  const fresh = await api.rpc(BOB, 'doc.freshness', { workspace_id: ws.workspace_id, entity_ids: ['sales'] })
  expect(fresh.items[0].status).toBe('unknown')
  expect(JSON.stringify(fresh)).not.toContain('原始销售数据')
})

test('内核架构改进 D22: the owner configures hosted execution; a reader only sees it; whose AI a run uses is no setting', async ({ page, browser, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `hosted ${Date.now()}`)
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', allow: ['read'] }] })
  await openWorkspace(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-left-permissions').click()
  // a run uses the AI of where it runs (local: the starter's own, hosted: the owner's): nothing to choose
  await expect(page.getByTestId('aiws-hosted-enabled')).toBeVisible()
  await expect(page.getByTestId('aiws-wish-aicc-mode')).toHaveCount(0)

  const bobPage = await (await browser.newContext()).newPage()
  await openWorkspace(bobPage, BOB, ws.workspace_id)
  await bobPage.getByTestId('aiws-top-sources').click()
  await bobPage.getByTestId('aiws-left-permissions').click()
  // the owner names the Agent and the wishes (T3-7); a reader cannot change it
  await page.getByTestId('aiws-hosted-enabled').check()
  await page.getByTestId('aiws-hosted-agent').fill('jarvis.test.buckyos.io')
  await page.getByTestId('aiws-hosted-wish-wish-analysis').check()
  await page.getByTestId('aiws-hosted-save').click()
  await expect.poll(async () => (await api.rpc(ALICE, 'ws.get_info', { workspace_id: ws.workspace_id })).hosted_exec).toMatchObject({ enabled: true, agent_id: 'jarvis.test.buckyos.io', allowed_wishes: ['wish-analysis'] })
  await expect(bobPage.getByTestId('aiws-hosted-save')).toBeDisabled()
  const hostedRefused = await api.rpc(BOB, 'ws.set_hosted_exec', { workspace_id: ws.workspace_id, config: { enabled: false } })
  expect(hostedRefused.error.code).toBe('PERMISSION_DENIED')
})

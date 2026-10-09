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
  const grants = await api.rpc(ALICE, 'ws.list_grants', { workspace_id: ws.workspace_id })
  const bobs = grants.grants.filter((g: { subject: string }) => g.subject === 'bob').map((g: { scope_entity_id: string | null }) => g.scope_entity_id).sort()
  expect(bobs).toEqual([null, 'sf-detail', 'sf-detail-content'].sort())
  // bob: edits on the detail canvas are accepted (union), first-level data edits are refused
  const bobGrants = await api.rpc(BOB, 'ws.list_grants', { workspace_id: ws.workspace_id })
  expect(bobGrants.complete).toBe(false)
  expect(bobGrants.grants.every((g: { subject: string }) => g.subject === 'bob')).toBe(true)
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
  const after = await api.rpc(ALICE, 'ws.list_grants', { workspace_id: ws.workspace_id })
  expect(after.grants.filter((g: { subject: string }) => g.subject === 'bob')).toHaveLength(1)
  expect((await api.rpc(BOB, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'sales' })).ok).toBe(true)
  // a principal reading only a subtree learns nothing about referrers or dependencies elsewhere
  await api.rpc(ALICE, 'ws.revoke', { workspace_id: ws.workspace_id, subject: 'bob' })
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', scope_entity_id: 'sf-detail-content', capabilities: ['read'] })
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

/* Phase two §13.1 UI14, in the permission editor of UI 框架 PRD §8.3: rules by path — the workspace, and a canvas with
 * the data folder it holds — union semantics, and no leaks of names, referrers or dependency details to a restricted
 * principal. */

import type { Page } from '@playwright/test'
import { expect, openWorkspace, test } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'

/** The data mode with the permission editor (§8). */
async function openEditor(page: Page, token: string, workspaceId: string) {
  await openWorkspace(page, token, workspaceId)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-dm-perm-node').click()
  await expect(page.getByTestId('aiws-permissions')).toBeVisible()
}

/** A rule added in the rule list: who, the path, a preset. */
async function addRule(page: Page, subject: string, path: string, preset: string) {
  await page.getByTestId('aiws-perm-rule-add').click()
  const rule = page.getByTestId('aiws-perm-rule-editor')
  await rule.getByTestId('aiws-perm-subject').selectOption(subject)
  await rule.getByTestId('aiws-perm-path').fill(path)
  await expect(rule.getByTestId('aiws-perm-path-check')).toHaveAttribute('data-state', 'ok')
  await rule.getByTestId(`aiws-perm-preset-${preset}`).click()
  await rule.getByTestId('aiws-perm-rule-save').click()
  await expect(rule).toHaveCount(0)
}

test('UI14 workspace and canvas rules by path, and what a restricted principal can see', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `perm ${Date.now()}`)
  await openEditor(page, ALICE, ws.workspace_id)
  const editor = page.getByTestId('aiws-permissions')
  // the owner holds everything without a rule
  await expect(editor.locator('[data-testid="aiws-perm-group"][data-subject="@owner"]')).toContainText('全部能力')
  await editor.getByTestId('aiws-perm-tab-rules').click()
  // bob becomes a reader of the workspace, and an editor of the detail canvas only (two entries: the Surface and the
  // data folder it holds)
  await addRule(page, 'bob', '/', 'reader')
  await addRule(page, 'bob', '/surfaces/数据明细', 'editor')
  const row = (path: string) => editor.locator(`[data-testid="aiws-perm-rule"][data-subject="bob"][data-resource="${path}"]`)
  await expect(row('/').getByTestId('aiws-perm-rule-allow').locator('[data-cap]')).toHaveCount(1)
  await expect(row('/surfaces/数据明细').getByTestId('aiws-perm-rule-allow').locator('[data-cap]')).toHaveCount(6)
  const content = editor.locator('[data-testid="aiws-perm-rule"][data-subject="bob"][data-entity-id="sf-detail-content"]')
  await expect(content.getByTestId('aiws-perm-rule-allow').locator('[data-cap]')).toHaveCount(6)
  // the entries behind the editor (acl.query by: all): the workspace, the canvas and its content folder
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
  // deleting the canvas rule deletes its data folder's too; the workspace rule still makes bob a reader
  await row('/surfaces/数据明细').getByTestId('aiws-perm-rule-edit').click()
  await expect(page.getByTestId('aiws-perm-rule-pair')).toBeChecked()
  await page.getByTestId('aiws-perm-rule-delete').click()
  await expect(row('/surfaces/数据明细')).toHaveCount(0)
  await expect(content).toHaveCount(0)
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

test('内核架构改进 D22: the owner configures hosted execution; a reader cannot change it; whose AI a run uses is no setting', async ({ page, browser, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `hosted ${Date.now()}`)
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', allow: ['read'] }] })
  await openEditor(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-perm-tab-hosted').click()
  // a run uses the AI of where it runs (local: the starter's own, hosted: the owner's): nothing to choose
  await expect(page.getByTestId('aiws-hosted-enabled')).toBeVisible()
  await expect(page.getByTestId('aiws-wish-aicc-mode')).toHaveCount(0)

  // a reader sees only its own permissions: no hosted execution to change (the owner's own, 权限需求 R17)
  const bobPage = await (await browser.newContext()).newPage()
  await openEditor(bobPage, BOB, ws.workspace_id)
  await expect(bobPage.getByTestId('aiws-perm-mine')).toBeVisible()
  // the owner names the Agent and the wishes (T3-7); a reader cannot change it
  await page.getByTestId('aiws-hosted-enabled').check()
  await page.getByTestId('aiws-hosted-agent').fill('jarvis.test.buckyos.io')
  await page.getByTestId('aiws-hosted-wish-wish-analysis').check()
  await page.getByTestId('aiws-hosted-save').click()
  await expect.poll(async () => (await api.rpc(ALICE, 'ws.get_info', { workspace_id: ws.workspace_id })).hosted_exec).toMatchObject({ enabled: true, agent_id: 'jarvis.test.buckyos.io', allowed_wishes: ['wish-analysis'] })
  await expect(bobPage.getByTestId('aiws-hosted-save')).toHaveCount(0)
  const hostedRefused = await api.rpc(BOB, 'ws.set_hosted_exec', { workspace_id: ws.workspace_id, config: { enabled: false } })
  expect(hostedRefused.error.code).toBe('PERMISSION_DENIED')
})

/* Sources and publication by path (内核架构改进 M4): a manager publishes a table; another workspace inserts the link
 * as data and the copy follows the publication (a refresh run); the copy is read-only; the publisher sees who reads it. */
import { expect, openWorkspace, test, treeMenu } from './fixtures'

const ALICE = 'tok-alice'

test('M4: publish a table, insert its link elsewhere, the copy follows and stays read-only, the publisher lists the referrer', async ({ page, api }) => {
  test.slow()
  const a = await api.demo(ALICE, 'quarterly', `publisher ${Date.now()}`)
  const b = (await api.rpc(ALICE, 'ws.create', { title: `consumer ${Date.now()}` })) as { workspace_id: string; epoch: string }
  // the publisher
  await openWorkspace(page, ALICE, a.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-tree-sales').click()
  await expect(page.getByTestId('aiws-publication')).toHaveAttribute('data-published', 'false')
  await page.getByTestId('aiws-publish').click()
  await expect(page.getByTestId('aiws-publication')).toHaveAttribute('data-published', 'true')
  const link = (await page.getByTestId('aiws-publish-link').textContent()) ?? ''
  // the publication link is on the workspace's own origin (the backend knows its public base)
  expect(link).toContain(`aiws-${a.workspace_id.replace(/^ws_/, '')}.localhost`)
  const published = await api.rpc(ALICE, 'ws.list_publications', { workspace_id: a.workspace_id })
  expect(published.publications[0].entity_id).toBe('sales')
  const sourceRows = (await api.table(ALICE, a.workspace_id, { source_id: 'sales', limit: 100 })).rows.length
  // the consumer inserts the link as data: created, then refreshed at once
  await openWorkspace(page, ALICE, b.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await treeMenu(page, null, 'aiws-dm-menu-new', 'aiws-dm-new-link')
  await page.getByTestId('aiws-new-link').fill(link)
  await page.getByTestId('aiws-new-data').click()
  await expect(page.getByTestId('aiws-source')).toHaveAttribute('data-kind', 'workspace', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-source')).toHaveAttribute('data-mode', 'live')
  await expect(page.getByTestId('aiws-row')).toHaveCount(sourceRows, { timeout: 30_000 })
  const copy = (await api.rpc(ALICE, 'doc.outline', { workspace_id: b.workspace_id })).entities.find((e: { source?: unknown }) => e.source)
  expect(copy.source.entity_id).toBe('sales')
  // read-only: the service refuses an edit of sourced content
  const info = await api.rpc(ALICE, 'ws.get_info', { workspace_id: b.workspace_id })
  const record = (await api.table(ALICE, b.workspace_id, { source_id: copy.entity_id, limit: 1 })).rows[0]
  const refused = await api.rpc(ALICE, 'doc.commit', { workspace_id: b.workspace_id, protocol_version: '1.0', epoch: info.epoch, idempotency_key: `edit-${Date.now()}`, session_id: 's1',
    operations: [{ op: 'table.delete_records', source_id: copy.entity_id, records: [{ record_id: record.record_id, expect: { rev: record.rev } }] }] })
  expect(refused.status).toBe('rejected')
  expect(refused.errors[0].sub_code).toBe('SOURCE_READ_ONLY')
  // the publisher's change reaches the copy on the next refresh
  const cell = await api.cell(ALICE, a.workspace_id, 'sales', 's-1', 'revenue')
  await api.commit(ALICE, a, [{ op: 'table.set_values', source_id: 'sales', values: [{ record_id: 's-1', field_id: 'revenue', value: 777777, expect: { rev: cell.rev } }] }])
  await page.getByTestId('aiws-source-refresh').click()
  await expect(page.getByTestId('aiws-source-run')).toHaveAttribute('data-state', 'succeeded', { timeout: 30_000 })
  expect((await api.cell(ALICE, b.workspace_id, copy.entity_id, 's-1', 'revenue')).value).toBe(777777)
  // the publisher lists its referrer
  await openWorkspace(page, ALICE, a.workspace_id)
  // the workspace reopens in the mode it was left in (sources): the button is only there on the canvas
  if (await page.getByTestId('aiws-top-sources').isVisible()) await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-tree-sales').click()
  await expect(page.getByTestId('aiws-referrer')).toHaveCount(1)
  await expect(page.getByTestId('aiws-referrer')).toContainText(b.workspace_id)
})

test('contacts catalog discloses the copy and rebind can be cancelled without changing local rows', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `contacts ${Date.now()}`)
  await openWorkspace(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await treeMenu(page, null, 'aiws-dm-menu-new', 'aiws-dm-zone-sources', 'aiws-dm-new-contacts')
  const dialog = page.getByTestId('aiws-contacts-dialog')
  await expect(dialog).toContainText('alice')
  await expect(dialog).toContainText('contacts ')
  await expect(dialog).toContainText('DID')
  const mode = page.getByTestId('aiws-contacts-mode')
  await expect(mode).toHaveValue('auto')
  await mode.selectOption('manual')
  await mode.selectOption('once')
  const submitted = page.waitForRequest((request) => request.method() === 'POST' && request.url().endsWith('/kapi/aiworkspace') && request.postDataJSON()?.method === 'source.bind_contacts')
  await page.getByTestId('aiws-contacts-submit').click()
  const { params } = (await submitted).postDataJSON()
  expect(params).toEqual({
    workspace_id: ws.workspace_id, epoch: ws.epoch, entity_id: expect.any(String), idempotency_key: expect.any(String), mode: 'once',
    create: { parent_id: 'data', order_key: expect.any(String), name: expect.any(String) },
  })
  await expect(dialog.getByRole('alert')).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(dialog).toHaveCount(0)
  const before = await api.table(ALICE, ws.workspace_id, { source_id: 'sales', limit: 100 })
  await page.getByTestId('aiws-tree-sales').click()
  await page.getByTestId('aiws-contacts-rebind').click()
  await expect(dialog.locator('.aiws-warning')).toBeVisible()
  await expect(mode.locator('option[value="once"]')).toHaveCount(0)
  await page.keyboard.press('Escape')
  const after = await api.table(ALICE, ws.workspace_id, { source_id: 'sales', limit: 100 })
  expect(after.rows).toEqual(before.rows)
})

test('a copied contacts descriptor needs owner authorization; a local table can edit tag arrays and open the original record', async ({ page, api }) => {
  const ws = await api.rpc(ALICE, 'ws.create', { title: `shared contacts ${Date.now()}` })
  const original = 'obj://msg-center/owners/did:bns:alice/contacts/did:bns:carol'
  const seeded = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'imported', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'a', name: 'Imported contacts', payload: {
      source: { kind: 'builtin', provider: 'contacts', def: { meaning: 'Contacts', primary_key: ['did'] } },
    } },
    { op: 'entity.create', entity_id: 'local', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'b', name: 'Local copy', payload: {
      fields: [{ field_id: 'tags', name: 'Tags', type: 'json' }, { field_id: 'contact_ref', name: 'Original', type: 'text' }],
    } },
    { op: 'table.insert_records', source_id: 'local', records: [{ record_id: 'one', values: { tags: ['team'], contact_ref: original } }] },
  ])
  expect(seeded.status, JSON.stringify(seeded)).toBe('accepted')
  await openWorkspace(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-tree-imported').click()
  await expect(page.getByTestId('aiws-contacts-unbound')).toBeVisible()
  await expect(page.getByTestId('aiws-source-refresh')).toBeDisabled()
  await expect(page.getByTestId('aiws-contacts-rebind')).toBeEnabled()
  await page.getByTestId('aiws-tree-local').click()
  const tags = page.getByTestId('aiws-cell-one-tags')
  await tags.getByRole('button').click()
  await tags.getByRole('textbox').fill('["updated"]')
  await tags.getByRole('textbox').press('Control+Enter')
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'local', 'one', 'tags')).value).toEqual(['updated'])
  const link = page.getByTestId('aiws-cell-one-contact_ref').getByRole('link')
  const href = new URL((await link.getAttribute('href'))!)
  expect(decodeURIComponent(href.pathname)).toBe('/kapi/msg-center/obj/owners/did:bns:alice/contacts/did:bns:carol/props/record')
  await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', resource: '/', allow: ['read', 'structure'] }] })
  await openWorkspace(page, 'tok-bob', ws.workspace_id)
  if (await page.getByTestId('aiws-top-sources').isVisible()) await page.getByTestId('aiws-top-sources').click()
  await treeMenu(page, null, 'aiws-dm-menu-new', 'aiws-dm-zone-sources')
  await expect(page.getByTestId('aiws-dm-new-contacts')).toBeDisabled()
  await page.keyboard.press('Escape')
  await page.getByTestId('aiws-tree-imported').click()
  await expect(page.getByTestId('aiws-contacts-unbound')).toBeVisible()
  await expect(page.getByTestId('aiws-contacts-rebind')).toHaveCount(0)
})

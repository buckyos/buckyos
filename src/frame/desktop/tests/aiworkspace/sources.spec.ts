/* Sources and publication by path (内核架构改进 M4): a manager publishes a table; another workspace inserts the link
 * as data and the copy follows the publication (a refresh run); the copy is read-only; the publisher sees who reads it. */
import { expect, openWorkspace, test } from './fixtures'

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
  expect(link).toContain(`/kapi/aiworkspace/pub/${a.workspace_id}/`)
  const published = await api.rpc(ALICE, 'ws.list_publications', { workspace_id: a.workspace_id })
  expect(published.publications[0].entity_id).toBe('sales')
  const sourceRows = (await api.rpc(ALICE, 'doc.query', { workspace_id: a.workspace_id, source_id: 'sales', limit: 100 })).rows.length
  // the consumer inserts the link as data: created, then refreshed at once
  await openWorkspace(page, ALICE, b.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-new-kind').selectOption('link')
  await page.getByTestId('aiws-new-link').fill(link)
  await page.getByTestId('aiws-new-data').click()
  await expect(page.getByTestId('aiws-source')).toHaveAttribute('data-kind', 'workspace', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-source')).toHaveAttribute('data-mode', 'live')
  await expect(page.getByTestId('aiws-row')).toHaveCount(sourceRows, { timeout: 30_000 })
  const copy = (await api.rpc(ALICE, 'doc.outline', { workspace_id: b.workspace_id })).entities.find((e: { source?: unknown }) => e.source)
  expect(copy.source.entity_id).toBe('sales')
  // read-only: the service refuses an edit of sourced content
  const info = await api.rpc(ALICE, 'ws.get_info', { workspace_id: b.workspace_id })
  const record = (await api.rpc(ALICE, 'doc.query', { workspace_id: b.workspace_id, source_id: copy.entity_id, limit: 1 })).rows[0]
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

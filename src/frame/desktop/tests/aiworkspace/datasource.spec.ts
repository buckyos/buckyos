/* Data source state and a table view's row limit (AI Native 余下任务 §7.2: B0-2, B0-3, B7-3): a data source defined before
 * its data shows as pending — its definition, never an empty table — and its failed first attempt with the reason; a
 * table view Block may show only its first rows, and "show all rows" takes the limit away. */
import { expect, openWorkspace, test } from './fixtures'

const ALICE = 'tok-alice'

test('B0-3: a data source without data is pending, shows its definition and its last attempt', async ({ page, api }) => {
  const ws = (await api.rpc(ALICE, 'ws.create', { title: `quotes ${Date.now()}` })) as { workspace_id: string; epoch: string }
  await api.commit(ALICE, ws, [{ op: 'entity.create', entity_id: 'quotes', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'q', name: 'quotes',
    payload: { fields: [{ field_id: 'symbol', name: 'Symbol', type: 'text' }, { field_id: 'close', name: 'Close', type: 'number' }],
      source: { kind: 'zone', path: '/home/alice/sync/quotes.json', def: { meaning: 'Daily closing prices', primary_key: ['symbol'], limit: 100 } } } }])
  await openWorkspace(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-tree-quotes').click()
  const panel = page.getByTestId('aiws-source')
  await expect(panel).toHaveAttribute('data-state', 'pending', { timeout: 30_000 })
  await expect(panel).toHaveAttribute('data-kind', 'zone')
  await expect(page.getByTestId('aiws-source-pending')).toBeVisible()
  await expect(page.getByTestId('aiws-source-meaning')).toContainText('Daily closing prices')
  await expect(page.locator('[data-testid^="aiws-table-pending-"]').first()).toBeVisible()
  // this deployment reads no Zone files: the first sync fails, says why, and the source stays pending
  await page.getByTestId('aiws-source-refresh').click()
  await expect(page.getByTestId('aiws-source-attempt')).toHaveAttribute('data-status', 'failed', { timeout: 30_000 })
  await expect(page.getByTestId('aiws-source-attempt')).toContainText('not readable')
  await expect(panel).toHaveAttribute('data-state', 'pending')
})

test('B7-3: a table view shows its first rows, and all of them again', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `rows ${Date.now()}`)
  const rev = ((await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'cell-all-tasks' })).content.key_revs.row_limit ?? 0) as number
  await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'cell-all-tasks', keys: [{ key: 'row_limit', value: 2, expect: { rev } }] }])
  await openWorkspace(page, ALICE, ws.workspace_id)
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  await expect(all.getByTestId('aiws-row')).toHaveCount(2)
  await expect(all.getByTestId('aiws-table-count-cell-all-tasks')).toHaveAttribute('data-limited', 'true')
  await all.getByTestId('aiws-table-menu-cell-all-tasks').click()
  await page.getByTestId('aiws-ctl-menu-table-rows').click()
  await expect(all.getByTestId('aiws-row')).toHaveCount(5)
  await expect.poll(async () => (await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'cell-all-tasks' })).content.payload.row_limit ?? null).toBe(null)
})

/* 内核架构改进 M5: the draft protocol of a form Block (D13), formulas shown evaluated and materialized on request (D18),
 * duplicating a canvas with same-canvas semantics and pinning own data into the data tree (D05). */
import { expect, expectAllCommitted, openCanvas, openWorkspace, test } from './fixtures'

const ALICE = 'tok-alice'

test('T5-1: a note draft recorded with its idempotency key — an accepted submission is cleared on another device, an unsent one is resent under the same key', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `drafts ${Date.now()}`)
  const created = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'note-a', type_id: 'buckyos.annotation', parent_id: 'data', order_key: 'za', name: 'A', payload: { kind: 'note', body: '原文 A', style: { color: '#fff2cc' } } },
    { op: 'entity.create', entity_id: 'note-b', type_id: 'buckyos.annotation', parent_id: 'data', order_key: 'zb', name: 'B', payload: { kind: 'note', body: '原文 B', style: { color: '#fff2cc' } } },
  ])
  expect(created.status, JSON.stringify(created)).toBe('accepted')
  const info = await api.rpc(ALICE, 'ws.get_info', { workspace_id: ws.workspace_id })
  // device A submitted note-a with key K, then failed to clear its draft
  const rev = (await api.read(ALICE, ws.workspace_id, 'note-a')).content.key_revs.body
  const accepted = await api.rpc(ALICE, 'doc.commit', { workspace_id: ws.workspace_id, protocol_version: '1.0', epoch: info.epoch, idempotency_key: 'devA/draft/1', session_id: 'test-api',
    operations: [{ op: 'entity.set_keys', entity_id: 'note-a', keys: [{ key: 'body', value: '改好的 A', expect: { rev } }] }] })
  expect(accepted.status, JSON.stringify(accepted)).toBe('accepted')
  await api.rpc(ALICE, 'ws.set_user_state', { workspace_id: ws.workspace_id, entries: {
    'block:note-a/draft:body': { text: '改好的 A', at: '2026-10-23T00:00:00.000Z', pending_key: 'devA/draft/1', epoch: info.epoch },
    'block:note-b/draft:body': { text: '没发出去的 B', at: '2026-10-23T00:00:00.000Z', pending_key: 'devA/draft/2', epoch: info.epoch },
  } })
  const head = await api.headSeq(ALICE, ws.workspace_id)
  // device B opens note-a: the submission is found, the draft goes, nothing is resent
  await openWorkspace(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-tree-note-a').click()
  await expect(page.getByTestId('aiws-note-note-a')).toContainText('改好的 A')
  await expect.poll(async () => (await api.rpc(ALICE, 'ws.get_user_state', { workspace_id: ws.workspace_id })).entries['block:note-a/draft:body'] ?? null).toBeNull()
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  // note-b: its draft was never recorded by the service — it is offered again and goes out under the same key
  await page.getByTestId('aiws-tree-note-b').click()
  const editor = page.getByTestId('aiws-note-note-b').locator('textarea')
  await expect(editor).toHaveValue('没发出去的 B')
  await editor.blur()
  await expect(page.getByTestId('aiws-note-note-b')).toContainText('没发出去的 B')
  await expect.poll(async () => (await api.rpc(ALICE, 'doc.get_submission', { workspace_id: ws.workspace_id, epoch: info.epoch, idempotency_key: 'devA/draft/2' })).status).toBe('accepted')
  expect((await api.read(ALICE, ws.workspace_id, 'note-b')).content.payload.body).toBe('没发出去的 B')
  await expect.poll(async () => (await api.rpc(ALICE, 'ws.get_user_state', { workspace_id: ws.workspace_id })).entries['block:note-b/draft:body'] ?? null).toBeNull()
})

test('T5-3: a $formula value is shown evaluated and materialized into a derived value on request', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `formula ${Date.now()}`)
  const cell = await api.cell(ALICE, ws.workspace_id, 'sales', 's-1', 'target')
  await api.commit(ALICE, ws, [{ op: 'table.set_values', source_id: 'sales', values: [{ record_id: 's-1', field_id: 'target', value: { $formula: { lang: 'aiws', version: '1', expr: '销售额 - 成本' } }, expect: { rev: cell.rev } }] }])
  expect((await api.cell(ALICE, ws.workspace_id, 'sales', 's-1', 'target')).value).toEqual({ $formula: { lang: 'aiws', version: '1', expr: '销售额 - 成本' } })
  await openWorkspace(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-top-sources').click()
  await page.getByTestId('aiws-tree-sales').click()
  const shown = page.getByTestId('aiws-cell-s-1-target')
  await expect(shown).toContainText('52000')
  await shown.getByTestId('aiws-cell-materialize').click()
  await expect.poll(async () => (await api.cell(ALICE, ws.workspace_id, 'sales', 's-1', 'target')).value).toBe(52000)
  const record = (await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'sales', selector: { kind: 'table_record', record_id: 's-1' } })).content
  expect(record.meta?.target?.derived?.program, JSON.stringify(record)).toBe('formula')
})

test('T5-4: duplicating a canvas clones its own data and references shared data; own data can be pinned into the data tree', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `dup ${Date.now()}`)
  // own data: a table created with its Block under the canvas's content folder
  await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'own-t', type_id: 'buckyos.table-source', parent_id: 'sf-analysis-content', order_key: 'zz', name: '画布自有表', payload: { fields: [{ field_id: 'k', name: '键', type: 'text' }] } },
    { op: 'entity.create', entity_id: 'blk-own', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'zz', placement: { x: 40, y: 900, w: 400, h: 200 }, payload: { view: { type: 'table' }, source_ref: { entity_id: 'own-t' } } },
  ])
  const before = (await api.rpc(ALICE, 'doc.outline', { workspace_id: ws.workspace_id })).entities as { entity_id: string; type_id: string; parent_id?: string; source_id?: string | null; kind?: string }[]
  const blocksBefore = before.filter((e) => e.type_id === 'buckyos.cell' && e.parent_id === 'sf-analysis').length
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-more-sf-analysis').click()
  await page.getByTestId('aiws-surface-duplicate-sf-analysis').click()
  await expect.poll(async () => ((await api.rpc(ALICE, 'doc.outline', { workspace_id: ws.workspace_id })).entities as { kind?: string; parent_id?: string }[]).filter((e) => e.kind === 'surface').length).toBe(before.filter((e) => e.kind === 'surface').length + 1)
  const after = (await api.rpc(ALICE, 'doc.outline', { workspace_id: ws.workspace_id })).entities as { entity_id: string; type_id: string; parent_id?: string; source_id?: string | null; kind?: string; title?: string | null }[]
  const copy = after.find((e) => e.kind === 'surface' && e.entity_id !== 'sf-analysis' && e.title?.includes('副本'))!
  expect(copy).toBeTruthy()
  const copied = after.filter((e) => e.type_id === 'buckyos.cell' && e.parent_id === copy.entity_id)
  expect(copied.length).toBe(blocksBefore)
  // the shared sales table is referenced, the own table is cloned into the copy's own folder
  expect(copied.some((b) => b.source_id === 'sales')).toBe(true)
  const ownCopy = copied.find((b) => b.source_id && b.source_id !== 'own-t' && after.find((e) => e.entity_id === b.source_id)?.parent_id === `${copy.entity_id}-content`)
  expect(ownCopy).toBeTruthy()
  expect(copied.some((b) => b.source_id === 'own-t')).toBe(false)
  // pinning: the own table leaves the canvas and lives in the data tree
  await page.getByTestId('aiws-top-sources').click()
  await page.locator('[data-testid="aiws-tree-item"][data-entity-id="canvas-content"] .aiws-tree-toggle').click()
  await page.locator('[data-testid="aiws-tree-item"][data-entity-id="sf-analysis-content"] .aiws-tree-toggle').click()
  await page.getByTestId('aiws-tree-own-t').click()
  await expect(page.getByTestId('aiws-own-data')).toBeVisible()
  await page.getByTestId('aiws-data-pin').click()
  await expect.poll(async () => ((await api.rpc(ALICE, 'doc.outline', { workspace_id: ws.workspace_id })).entities as { entity_id: string; parent_id?: string }[]).find((e) => e.entity_id === 'own-t')?.parent_id).toBe('data')
  await expect(page.getByTestId('aiws-own-data')).toHaveCount(0)
})

test('T5-2: entering a lock_required editor takes the lock; a lock broken behind this window refuses the next edit at the editor and keeps it', async ({ page, api }) => {
  const ws = await api.sample(ALICE, `enter-lock ${Date.now()}`)
  const notesRev = (await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'notes' })).meta_rev
  const tasksRev = (await api.rpc(ALICE, 'doc.read', { workspace_id: ws.workspace_id, entity_id: 'tasks' })).meta_rev
  expect((await api.commit(ALICE, ws, [
    { op: 'entity.set_write_policy', entity_id: 'notes', policy: 'lock_required', expect: { rev: notesRev } },
    { op: 'entity.set_write_policy', entity_id: 'tasks', policy: 'lock_required', expect: { rev: tasksRev } },
  ])).status).toBe('accepted')
  await openWorkspace(page, ALICE, ws.workspace_id)
  if (await page.getByTestId('aiws-top-canvas').isVisible()) await page.getByTestId('aiws-top-canvas').click()
  const notes = page.getByTestId('aiws-richtext-notes')
  const lock = page.getByTestId('aiws-lock-notes')
  await expect(lock).toHaveAttribute('data-held', 'false')
  await expect(notes).toHaveAttribute('contenteditable', 'false')
  // the first pointer on the text asks for the lock: no "start editing" click
  await notes.locator('[data-block-id="n-end"]').click()
  await expect(lock).toHaveAttribute('data-held', 'true')
  await expect(notes).toHaveAttribute('contenteditable', 'true')
  await notes.locator('[data-block-id="n-end"]').click()
  await page.keyboard.press('End')
  await page.keyboard.type('进入即取锁。')
  await expectAllCommitted(page)
  expect(JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).toContain('进入即取锁。')
  // a table the same way: the first click on a cell
  const all = page.getByTestId('aiws-cell-frame-cell-all-tasks')
  await expect(all.getByTestId('aiws-cell-task-41-title')).toHaveAttribute('data-editable', 'false')
  await all.getByTestId('aiws-cell-task-41-title').click()
  await expect(all.getByTestId('aiws-lock-tasks')).toHaveAttribute('data-held', 'true')
  await expect(all.getByTestId('aiws-cell-task-41-title')).toHaveAttribute('data-editable', 'true')
  // the lock is broken behind this window: the next edit is refused, said at the editor, and kept
  await api.rpc(ALICE, 'lock.break', { workspace_id: ws.workspace_id, entity_id: 'notes' })
  await notes.locator('[data-block-id="n-end"]').click()
  await page.keyboard.press('End')
  await page.keyboard.type('失锁后。')
  await expect(page.getByTestId('aiws-lock-lost')).toBeVisible()
  await expect(lock).toHaveAttribute('data-held', 'false')
  await expect(page.getByTestId('aiws-save-summary')).toHaveAttribute('data-attention', '1')
  expect(JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).not.toContain('失锁后。')
  // entering again takes it back and the kept edit goes out
  await notes.locator('[data-block-id="n-end"]').click()
  await expect(lock).toHaveAttribute('data-held', 'true')
  await expectAllCommitted(page)
  await expect.poll(async () => JSON.stringify(await api.ast(ALICE, ws.workspace_id, 'notes'))).toContain('失锁后。')
})

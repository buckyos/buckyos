import { createHash } from 'node:crypto'
import type { Locator } from '@playwright/test'
import type { ChangesPage, EntityEnvelope, Operation, QueryPage, QueryParams, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { createSample, expect, openWorkspace, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>

function query(actor: Actor, ws: Workspace, params: QueryParams) {
  return actor.client.ok<QueryPage>('doc.query', { workspace_id: ws.workspace_id, ...params })
}

function recordIds(page: QueryPage) { return page.rows.map((row) => row.record_id) }

async function renderedIds(table: Locator) {
  return table.getByTestId('aiws-row').evaluateAll((rows) => rows.map((row) => row.getAttribute('data-record-id')))
}

async function changes(actor: Actor, ws: Workspace, after_seq: number) {
  return actor.client.ok<ChangesPage>('doc.get_changes', { workspace_id: ws.workspace_id, epoch: ws.epoch, after_seq, limit: 100 })
}

async function snapshot(actor: Actor, ws: Workspace) {
  const [info, checkpoint, outline, ast, collab, status] = await Promise.all([
    actor.client.info(ws),
    actor.client.ok<{ content_root: string }>('doc.checkpoint', { workspace_id: ws.workspace_id }),
    actor.client.ok<{ entities: EntityEnvelope[] }>('doc.outline', { workspace_id: ws.workspace_id }),
    actor.client.ast(ws),
    actor.client.ok<{ snapshot: string }>('doc.get_collab_state', { workspace_id: ws.workspace_id, entity_id: 'notes' }),
    actor.client.cell(ws, 'task-42', 'status'),
  ])
  return {
    head_seq: info.head_seq, head_commit_id: info.head_commit_id,
    content_root: checkpoint.content_root, entities: outline.entities, ast, status,
    collab_sha256: createHash('sha256').update(collab.snapshot).digest('hex'),
  }
}

test('[logic-views] completion and reopening preserve source records and update saved and embedded views', async ({ owner }, info) => {
  const ws = await createSample(owner)
  const page = owner.page
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  const open = page.getByTestId('aiws-cell-frame-cell-open-tasks').getByTestId('aiws-table-cell-open-tasks')
  const embedded = page.getByTestId('aiws-embed-cell-open-tasks')
  const saved = () => query(owner, ws, { view_id: 'cell-open-tasks' })
  const assigned = () => query(owner, ws, {
    view_id: 'cell-open-tasks',
    filter: { op: 'cmp', field_id: 'owner', operator: 'eq', value: '林' },
  })
  const original = await query(owner, ws, { source_id: 'tasks' })
  expect(recordIds(original).sort()).toEqual(['task-40', 'task-41', 'task-42', 'task-43', 'task-44'])
  expect(recordIds(await saved())).toEqual(['task-41', 'task-42', 'task-44', 'task-43'])
  expect(recordIds(await assigned())).toEqual(['task-42'])
  const viewBefore = await owner.client.ok<{ content: unknown }>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'cell-open-tasks' })
  const status = await owner.client.cell(ws, 'task-42', 'status')
  const due = await owner.client.cell(ws, 'task-44', 'due')
  const before = await owner.client.info(ws)
  const completed = await owner.client.commit(ws, [{
    op: 'table.set_values', source_id: 'tasks', values: [
      { record_id: 'task-42', field_id: 'status', value: 'option-done', expect: { rev: status.rev } },
      { record_id: 'task-44', field_id: 'due', value: '2026-09-01', expect: { rev: due.rev } },
    ],
  }])
  expect(completed).toMatchObject({ status: 'accepted', seq: before.head_seq + 1 })
  const filtered = await saved()
  expect(recordIds(filtered)).toEqual(['task-44', 'task-41', 'task-43'])
  expect(filtered.total).toBe(3)
  expect(filtered.rows.every((row) => !('budget' in row.values) && !('owner' in row.values))).toBe(true)
  expect(recordIds(await assigned())).toEqual([])
  const full = await query(owner, ws, { source_id: 'tasks' })
  expect(full.total).toBe(5)
  expect(recordIds(full).sort()).toEqual(recordIds(original).sort())
  expect(full.rows.find((row) => row.record_id === 'task-42')?.values.status).toBe('option-done')
  for (const table of [open, embedded]) {
    await expect.poll(() => renderedIds(table)).toEqual(['task-44', 'task-41', 'task-43'])
  }
  await expect(all.getByTestId('aiws-row')).toHaveCount(5)
  await screenshot(page, info, 'logic-task-completed')

  const current = await owner.client.cell(ws, 'task-42', 'status')
  const reopened = await owner.client.commit(ws, [{
    op: 'table.set_values', source_id: 'tasks',
    values: [{ record_id: 'task-42', field_id: 'status', value: 'option-open', expect: { rev: current.rev } }],
  }])
  expect(reopened).toMatchObject({ status: 'accepted', seq: before.head_seq + 2 })
  const expected = ['task-44', 'task-41', 'task-42', 'task-43']
  expect(recordIds(await saved())).toEqual(expected)
  expect(recordIds(await assigned())).toEqual(['task-42'])
  const viewAfter = await owner.client.ok<{ content: unknown }>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'cell-open-tasks' })
  expect(viewAfter.content).toEqual(viewBefore.content)
  const events = await changes(owner, ws, before.head_seq)
  expect(events.changes).toHaveLength(2)
  expect(events.head_seq).toBe(before.head_seq + 2)
  await page.reload()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  for (const table of [open, embedded]) await expect.poll(() => renderedIds(table)).toEqual(expected)
  await expect(all.getByTestId('aiws-row')).toHaveCount(5)
  await expect(all.getByTestId('aiws-cell-task-42-status')).toContainText('进行中')
  await screenshot(page, info, 'logic-task-reopened')
  await info.attach('logic-views-verdict', {
    body: Buffer.from(JSON.stringify({ source_records: recordIds(full).sort(), completed_view: recordIds(filtered), reopened_view: expected, commits: events.changes.map((event) => event.seq), view_definition_unchanged: true })),
    contentType: 'application/json',
  })
})

test('[logic-atomic] an invalid final operation rolls back tree, rich text, table and history before a corrected batch commits once', async ({ owner }, info) => {
  const ws = await createSample(owner)
  const before = await snapshot(owner, ws)
  const batch = (value: string): Operation[] => [
    { op: 'tree.place', entity_id: 'cell-notes', order_key: 'e5' },
    {
      op: 'richtext.insert_blocks', entity_id: 'notes', position: { after: 'n-title' },
      blocks: [{ type: 'paragraph', attrs: { block_id: 'logic-batch-block' }, content: [{ type: 'text', text: '批次内新增：只有整批成功才应出现。' }] }],
    },
    {
      op: 'table.set_values', source_id: 'tasks',
      values: [{ record_id: 'task-42', field_id: 'status', value, expect: { rev: before.status.rev } }],
    },
  ]
  const rejected = await owner.client.commit(ws, batch('option-does-not-exist'))
  expect(rejected).toMatchObject({ status: 'rejected', code: 'INVALID_SCHEMA', errors: [expect.objectContaining({ op_index: 2 })] })
  expect(await snapshot(owner, ws)).toEqual(before)
  const rejectedEvents = await changes(owner, ws, before.head_seq)
  expect(rejectedEvents.changes).toEqual([])
  expect(rejectedEvents.head_seq).toBe(before.head_seq)
  await openWorkspace(owner, ws)
  await expect(owner.page.getByTestId('aiws-richtext-notes')).not.toContainText('批次内新增')
  await expect(owner.page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-42-status')).toContainText('进行中')
  await screenshot(owner.page, info, 'logic-batch-rejected')

  const accepted = await owner.client.commit(ws, batch('option-done'))
  expect(accepted).toMatchObject({ status: 'accepted', seq: before.head_seq + 1, replayed: false })
  if (accepted.status !== 'accepted') throw new Error('corrected batch was not accepted')
  const after = await snapshot(owner, ws)
  expect(after.head_seq).toBe(before.head_seq + 1)
  expect(after.head_commit_id).toBe(accepted.commit_id)
  expect(after.content_root).not.toBe(before.content_root)
  expect(after.collab_sha256).not.toBe(before.collab_sha256)
  expect(after.entities.find((entity) => entity.entity_id === 'cell-notes')?.order_key).toBe('e5')
  const inserted = after.ast.content?.filter((node) => node.attrs?.block_id === 'logic-batch-block')
  expect(inserted).toHaveLength(1)
  expect(JSON.stringify(inserted)).toContain('批次内新增：只有整批成功才应出现。')
  expect(after.status).toMatchObject({ value: 'option-done', rev: accepted.seq })
  const acceptedEvents = await changes(owner, ws, before.head_seq)
  expect(acceptedEvents.changes).toHaveLength(1)
  expect(acceptedEvents.changes[0]).toMatchObject({ seq: accepted.seq, commit_id: accepted.commit_id })
  expect(acceptedEvents.changes[0].touched).toEqual(expect.arrayContaining(['cell-notes', 'notes', 'tasks'].map((entity_id) => expect.objectContaining({ entity_id }))))
  await owner.page.reload()
  await expect(owner.page.getByTestId('aiws-richtext-notes')).toContainText('批次内新增：只有整批成功才应出现。')
  await expect(owner.page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-42-status')).toContainText('完成')
  await screenshot(owner.page, info, 'logic-batch-accepted')
  await info.attach('logic-atomic-verdict', {
    body: Buffer.from(JSON.stringify({ rejected, rejected_head: rejectedEvents.head_seq, rejected_changes: rejectedEvents.changes.length, original_root: before.content_root, committed_root: after.content_root, accepted, accepted_changes: acceptedEvents.changes.length })),
    contentType: 'application/json',
  })
})

import { unwrap } from '../../src/app/aiworkspace/api/client'
import type { ChangesPage, QueryPage, QueryParams, Result, SqlResult, WorkspaceInfo, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { createSample, expect, openWorkspace, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>

function query(actor: Actor, ws: Workspace, params: QueryParams) {
  return actor.client.table(ws, params)
}

async function root(actor: Actor, ws: Workspace) {
  return (await actor.client.ok<{ content_root: string }>('doc.checkpoint', { workspace_id: ws.workspace_id })).content_root
}

function collaboration(actor: Actor, ws: Workspace) {
  return actor.client.ok<{ lineage_id: string; snapshot: string }>('doc.get_collab_state', { workspace_id: ws.workspace_id, entity_id: 'notes' })
}

test('[logic-clone] a clone preserves local references but starts independent content, collaboration and permissions', async ({ owner, peer }, info) => {
  const source = await createSample(owner)
  await owner.client.ok('ws.grant', { workspace_id: source.workspace_id, subject: peer.userId, capabilities: ['read', 'export'] })
  expect((await peer.client.info(source)).workspace_id).toBe(source.workspace_id)
  const sourceInfo = await owner.client.info(source)
  const originalRoot = await root(owner, source)
  const originalAst = await owner.client.ast(source)
  const originalCollab = await collaboration(owner, source)
  const copy = await owner.client.ok<Workspace & { content_root: string; cloned_from: { workspace_id: string; snapshot: string } }>('ws.clone', { workspace_id: source.workspace_id })
  owner.client.owned.add(copy.workspace_id)
  expect(copy.workspace_id).not.toBe(source.workspace_id)
  expect(copy.content_root).toBe(originalRoot)
  expect(await root(owner, copy)).toBe(originalRoot)
  expect(copy.cloned_from.workspace_id).toBe(source.workspace_id)
  const copyInfo = await owner.client.info(copy)
  expect(copyInfo.title).toBe(owner.client.prefix)
  expect(copyInfo.cloned_from).toEqual(copy.cloned_from)
  expect(await owner.client.ast(copy)).toEqual(originalAst)
  const copyCollab = await collaboration(owner, copy)
  expect(copyCollab.lineage_id).not.toBe(originalCollab.lineage_id)
  expect(await peer.client.call<Result<WorkspaceInfo>>('ws.get_info', { workspace_id: copy.workspace_id })).toMatchObject({ ok: false, error: { code: 'NOT_FOUND' } })
  expect((await peer.client.list()).workspaces.some((ws) => ws.workspace_id === copy.workspace_id)).toBe(false)
  const view = await owner.client.ok<{ content: { payload: { source_ref: { entity_id: string } } } }>('doc.read', { workspace_id: copy.workspace_id, entity_id: 'cell-open-tasks' })
  expect(view.content.payload.source_ref).toEqual({ entity_id: 'tasks', version: { mode: 'live_head' } })
  const resolved = await owner.client.ok<{ reference: { workspace_id: string; entity_id: string } }>('doc.resolve', {
    workspace_id: copy.workspace_id, reference: view.content.payload.source_ref,
  })
  expect(resolved.reference).toMatchObject({ workspace_id: copy.workspace_id, entity_id: 'tasks' })
  const note = await owner.client.ok<{ content: { anchor: { state: string } } }>('doc.read', { workspace_id: copy.workspace_id, entity_id: 'note-budget' })
  expect(note.content.anchor.state).toBe('resolved')

  const copyTitle = '副本的独立任务'
  const copyNote = 'Clone 后仅写入副本的内容。'
  const copyCell = await owner.client.cell(copy, 'task-42', 'title')
  const changed = await owner.client.commit(copy, [
    { op: 'table.set_values', source_id: 'tasks', values: [{ record_id: 'task-42', field_id: 'title', value: copyTitle, expect: { rev: copyCell.rev } }] },
    { op: 'richtext.insert_blocks', entity_id: 'notes', position: { after: 'n-title' }, blocks: [{ type: 'paragraph', attrs: { block_id: 'copy-only-note' }, content: [{ type: 'text', text: copyNote }] }] },
  ])
  expect(changed).toMatchObject({ status: 'accepted', seq: copyInfo.head_seq + 1 })
  const copyRoot = await root(owner, copy)
  expect(copyRoot).not.toBe(originalRoot)
  expect(await root(owner, source)).toBe(originalRoot)
  expect(await owner.client.ast(source)).toEqual(originalAst)
  expect(await collaboration(owner, source)).toEqual(originalCollab)
  expect((await owner.client.info(source)).head_seq).toBe(sourceInfo.head_seq)
  const copyRows = await query(owner, copy, { view_id: 'cell-open-tasks' })
  expect(copyRows.rows.find((row) => row.record_id === 'task-42')?.values.title).toBe(copyTitle)

  const originalCell = await owner.client.cell(source, 'task-42', 'title')
  const originalTitle = '原件的独立任务'
  expect(await owner.client.commit(source, [{
    op: 'table.set_values', source_id: 'tasks',
    values: [{ record_id: 'task-42', field_id: 'title', value: originalTitle, expect: { rev: originalCell.rev } }],
  }])).toMatchObject({ status: 'accepted', seq: sourceInfo.head_seq + 1 })
  expect((await owner.client.cell(copy, 'task-42', 'title')).value).toBe(copyTitle)
  expect(await root(owner, copy)).toBe(copyRoot)
  expect((await peer.client.cell(source, 'task-42', 'title')).value).toBe(originalTitle)
  await openWorkspace(owner, copy)
  await expect(owner.page.getByTestId('aiws-richtext-notes')).toContainText(copyNote)
  await expect(owner.page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-42-title')).toContainText(copyTitle)
  await screenshot(owner.page, info, 'clone-independent-copy')
  await openWorkspace(owner, source)
  await expect(owner.page.getByTestId('aiws-richtext-notes')).not.toContainText(copyNote)
  await expect(owner.page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-42-title')).toContainText(originalTitle)
  await screenshot(owner.page, info, 'clone-independent-source')
  await info.attach('logic-clone-verdict', {
    body: Buffer.from(JSON.stringify({ source: source.workspace_id, clone: copy.workspace_id, initial_root: originalRoot, clone_root: copyRoot, source_lineage: originalCollab.lineage_id, clone_lineage: copyCollab.lineage_id, local_reference_workspace: resolved.reference.workspace_id, copied_grant: false, independent_titles: [originalTitle, copyTitle] })),
    contentType: 'application/json',
  })
})

test('[logic-pagination] SQL pages pinned to a head refuse a changed Workspace and restart without duplicates or missing records', async ({ owner }, info) => {
  const ws = await createSample(owner)
  const fields = (await owner.client.ok<{ content: { fields: { field_id: string; name: string }[] } }>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'tasks' })).content.fields
  const budget = fields.find((f) => f.field_id === 'budget')!.name
  // the paging a client does over SQL (核心 API 调整 §3.4): LIMIT / OFFSET, every page pinned to one head
  const sql = `SELECT _id, "${budget}#raw" FROM t ORDER BY ("${budget}" IS NULL), "${budget}#raw" COLLATE aiws_dec, _id LIMIT ? OFFSET ?`
  const page = (offset: number, head: { epoch: string; seq: number }) => owner.client.call<Result<SqlResult>>('doc.query', { workspace_id: ws.workspace_id, sql, params: [2, offset], tables: { t: 'tasks' }, expect_head: head })
  const before = await owner.client.info(ws)
  const pinned = { epoch: ws.epoch, seq: before.head_seq }
  const first = unwrap(await page(0, pinned))
  expect(first.rows.map((row) => row[0])).toEqual(['task-44', 'task-40'])
  expect((await owner.client.info(ws)).head_seq).toBe(before.head_seq)
  const cell = await owner.client.cell(ws, 'task-42', 'budget')
  const changed = await owner.client.commit(ws, [{
    op: 'table.set_values', source_id: 'tasks',
    values: [{ record_id: 'task-42', field_id: 'budget', value: '1', expect: { rev: cell.rev } }],
  }])
  expect(changed).toMatchObject({ status: 'accepted', seq: before.head_seq + 1 })
  const changedRoot = await root(owner, ws)
  const expired = await page(2, pinned)
  expect(expired).toMatchObject({ ok: false, error: { code: 'SNAPSHOT_EXPIRED', sub_code: 'HEAD_MISMATCH' } })
  expect(await root(owner, ws)).toBe(changedRoot)

  const now = { epoch: ws.epoch, seq: before.head_seq + 1 }
  const pages: SqlResult[] = []
  for (let offset = 0; offset < 8; offset += 2) {
    const p = unwrap(await page(offset, now))
    pages.push(p)
    if (p.rows.length < 2) break
  }
  expect(pages.map((p) => p.rows.length)).toEqual([2, 2, 1])
  const rows = pages.flatMap((p) => p.rows)
  const ids = rows.map((row) => row[0])
  expect(ids).toEqual(['task-42', 'task-44', 'task-40', 'task-41', 'task-43'])
  expect(new Set(ids).size).toBe(5)
  expect(rows.slice(0, 4).map((row) => row[1])).toEqual(['1.00', '9.50', '300.00', '800.00'])
  expect(pages[0].read[0].content_rev).toBe(before.head_seq + 1)
  const events = await owner.client.ok<ChangesPage>('doc.get_changes', { workspace_id: ws.workspace_id, epoch: ws.epoch, after_seq: before.head_seq, limit: 100 })
  expect(events.changes).toHaveLength(1)
  expect(events.head_seq).toBe(before.head_seq + 1)
  expect(await root(owner, ws)).toBe(changedRoot)
  await owner.page.reload()
  await expect(owner.page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-42-budget')).toContainText('1.00')
  await screenshot(owner.page, info, 'pagination-source-persisted')
  await info.attach('logic-pagination-verdict', {
    body: Buffer.from(JSON.stringify({ first_page: first.rows.map((row) => row[0]), pinned_head: pinned, rejection: expired, restarted_head: now, page_sizes: pages.map((p) => p.rows.length), records: ids, query_head_unchanged: events.head_seq, content_root: changedRoot })),
    contentType: 'application/json',
  })
})

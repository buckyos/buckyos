import type { ChangesPage, QueryPage, QueryParams, Result, WorkspaceInfo, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { createSample, expect, openWorkspace, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>

function query(actor: Actor, ws: Workspace, params: QueryParams) {
  return actor.client.ok<QueryPage>('doc.query', { workspace_id: ws.workspace_id, ...params })
}

async function root(actor: Actor, ws: Workspace) {
  return (await actor.client.ok<{ content_root: string }>('doc.checkpoint', { workspace_id: ws.workspace_id })).content_root
}

function collaboration(actor: Actor, ws: Workspace) {
  return actor.client.ok<{ lineage_id: string; snapshot: string }>('doc.get_collab_state', { workspace_id: ws.workspace_id, entity_id: 'notes' })
}

test('[logic-fork] a fork preserves local references but starts independent content, collaboration and permissions', async ({ owner, peer }, info) => {
  const source = await createSample(owner)
  await owner.client.ok('ws.grant', { workspace_id: source.workspace_id, subject: peer.userId, capabilities: ['read', 'export'] })
  expect((await peer.client.info(source)).workspace_id).toBe(source.workspace_id)
  const sourceInfo = await owner.client.info(source)
  const originalRoot = await root(owner, source)
  const originalAst = await owner.client.ast(source)
  const originalCollab = await collaboration(owner, source)
  const fork = await owner.client.ok<Workspace & { content_root: string; forked_from: { workspace_id: string } }>('ws.fork', { workspace_id: source.workspace_id })
  owner.client.owned.add(fork.workspace_id)
  expect(fork.workspace_id).not.toBe(source.workspace_id)
  expect(fork.content_root).toBe(originalRoot)
  expect(await root(owner, fork)).toBe(originalRoot)
  expect(fork.forked_from.workspace_id).toBe(source.workspace_id)
  const forkInfo = await owner.client.info(fork)
  expect(forkInfo.title).toBe(owner.client.prefix)
  expect(forkInfo.forked_from).toMatchObject({ workspace_id: source.workspace_id })
  expect(await owner.client.ast(fork)).toEqual(originalAst)
  const forkCollab = await collaboration(owner, fork)
  expect(forkCollab.lineage_id).not.toBe(originalCollab.lineage_id)
  expect(await peer.client.call<Result<WorkspaceInfo>>('ws.get_info', { workspace_id: fork.workspace_id })).toMatchObject({ ok: false, error: { code: 'NOT_FOUND' } })
  expect((await peer.client.list()).workspaces.some((ws) => ws.workspace_id === fork.workspace_id)).toBe(false)
  const view = await owner.client.ok<{ content: { payload: { source_ref: { entity_id: string } } } }>('doc.read', { workspace_id: fork.workspace_id, entity_id: 'cell-open-tasks' })
  expect(view.content.payload.source_ref).toEqual({ entity_id: 'tasks', version: { mode: 'live_head' } })
  const resolved = await owner.client.ok<{ reference: { workspace_id: string; entity_id: string } }>('doc.resolve', {
    workspace_id: fork.workspace_id, reference: view.content.payload.source_ref,
  })
  expect(resolved.reference).toMatchObject({ workspace_id: fork.workspace_id, entity_id: 'tasks' })
  const note = await owner.client.ok<{ content: { anchor: { state: string } } }>('doc.read', { workspace_id: fork.workspace_id, entity_id: 'note-budget' })
  expect(note.content.anchor.state).toBe('resolved')

  const copyTitle = '副本的独立任务'
  const copyNote = 'Fork 后仅写入副本的内容。'
  const forkCell = await owner.client.cell(fork, 'task-42', 'title')
  const changed = await owner.client.commit(fork, [
    { op: 'table.set_values', source_id: 'tasks', values: [{ record_id: 'task-42', field_id: 'title', value: copyTitle, expect: { rev: forkCell.rev } }] },
    { op: 'richtext.insert_blocks', entity_id: 'notes', position: { after: 'n-title' }, blocks: [{ type: 'paragraph', attrs: { block_id: 'fork-only-note' }, content: [{ type: 'text', text: copyNote }] }] },
  ])
  expect(changed).toMatchObject({ status: 'accepted', seq: forkInfo.head_seq + 1 })
  const forkRoot = await root(owner, fork)
  expect(forkRoot).not.toBe(originalRoot)
  expect(await root(owner, source)).toBe(originalRoot)
  expect(await owner.client.ast(source)).toEqual(originalAst)
  expect(await collaboration(owner, source)).toEqual(originalCollab)
  expect((await owner.client.info(source)).head_seq).toBe(sourceInfo.head_seq)
  const forkRows = await query(owner, fork, { view_id: 'cell-open-tasks' })
  expect(forkRows.rows.find((row) => row.record_id === 'task-42')?.values.title).toBe(copyTitle)

  const originalCell = await owner.client.cell(source, 'task-42', 'title')
  const originalTitle = '原件的独立任务'
  expect(await owner.client.commit(source, [{
    op: 'table.set_values', source_id: 'tasks',
    values: [{ record_id: 'task-42', field_id: 'title', value: originalTitle, expect: { rev: originalCell.rev } }],
  }])).toMatchObject({ status: 'accepted', seq: sourceInfo.head_seq + 1 })
  expect((await owner.client.cell(fork, 'task-42', 'title')).value).toBe(copyTitle)
  expect(await root(owner, fork)).toBe(forkRoot)
  expect((await peer.client.cell(source, 'task-42', 'title')).value).toBe(originalTitle)
  await openWorkspace(owner, fork)
  await expect(owner.page.getByTestId('aiws-richtext-notes')).toContainText(copyNote)
  await expect(owner.page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-42-title')).toContainText(copyTitle)
  await screenshot(owner.page, info, 'fork-independent-copy')
  await openWorkspace(owner, source)
  await expect(owner.page.getByTestId('aiws-richtext-notes')).not.toContainText(copyNote)
  await expect(owner.page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-42-title')).toContainText(originalTitle)
  await screenshot(owner.page, info, 'fork-independent-source')
  await info.attach('logic-fork-verdict', {
    body: Buffer.from(JSON.stringify({ source: source.workspace_id, fork: fork.workspace_id, initial_root: originalRoot, fork_root: forkRoot, source_lineage: originalCollab.lineage_id, fork_lineage: forkCollab.lineage_id, local_reference_workspace: resolved.reference.workspace_id, copied_grant: false, independent_titles: [originalTitle, copyTitle] })),
    contentType: 'application/json',
  })
})

test('[logic-pagination] consistent paging rejects a changed source and restarts without duplicates or missing records', async ({ owner }, info) => {
  const ws = await createSample(owner)
  const params: QueryParams = { source_id: 'tasks', sorts: [{ field_id: 'budget', direction: 'asc' }], consistency: 'snapshot', limit: 2 }
  const before = await owner.client.info(ws)
  const first = await query(owner, ws, params)
  expect(first.rows.map((row) => row.record_id)).toEqual(['task-44', 'task-40'])
  expect(first.total).toBe(5)
  expect(first.consistency).toBe('snapshot')
  expect(typeof first.next_cursor).toBe('string')
  if (!first.next_cursor) throw new Error('first page has no continuation cursor')
  expect((await owner.client.info(ws)).head_seq).toBe(before.head_seq)
  const budget = await owner.client.cell(ws, 'task-42', 'budget')
  const changed = await owner.client.commit(ws, [{
    op: 'table.set_values', source_id: 'tasks',
    values: [{ record_id: 'task-42', field_id: 'budget', value: '1', expect: { rev: budget.rev } }],
  }])
  expect(changed).toMatchObject({ status: 'accepted', seq: before.head_seq + 1 })
  const changedRoot = await root(owner, ws)
  const expired = await owner.client.call<Result<QueryPage>>('doc.query', { workspace_id: ws.workspace_id, ...params, cursor: first.next_cursor })
  expect(expired).toMatchObject({ ok: false, error: { code: 'SNAPSHOT_EXPIRED', retryable: true } })
  expect(await root(owner, ws)).toBe(changedRoot)

  const pages: QueryPage[] = []
  const seenCursors = new Set<string>()
  let cursor: string | undefined
  for (let index = 0; index < 4; index += 1) {
    const page = await query(owner, ws, { ...params, ...(cursor ? { cursor } : {}) })
    pages.push(page)
    expect(page.consistency).toBe('snapshot')
    expect(page.total).toBe(5)
    expect(page.content_rev).toBe(pages[0].content_rev)
    if (!page.next_cursor) break
    expect(seenCursors.has(page.next_cursor)).toBe(false)
    seenCursors.add(page.next_cursor)
    cursor = page.next_cursor
  }
  expect(pages.map((page) => page.rows.length)).toEqual([2, 2, 1])
  expect(pages.at(-1)?.next_cursor ?? null).toBeNull()
  const records = pages.flatMap((page) => page.rows)
  const ids = records.map((row) => row.record_id)
  expect(ids).toEqual(['task-42', 'task-44', 'task-40', 'task-41', 'task-43'])
  expect(new Set(ids).size).toBe(5)
  expect(records.slice(0, 4).map((row) => row.values.budget)).toEqual(['1.00', '9.50', '300.00', '800.00'])
  expect(pages[0].content_rev).not.toBe(first.content_rev)
  const events = await owner.client.ok<ChangesPage>('doc.get_changes', { workspace_id: ws.workspace_id, epoch: ws.epoch, after_seq: before.head_seq, limit: 100 })
  expect(events.changes).toHaveLength(1)
  expect(events.head_seq).toBe(before.head_seq + 1)
  expect(await root(owner, ws)).toBe(changedRoot)
  await owner.page.reload()
  await expect(owner.page.getByTestId('aiws-table-cell-all-tasks').getByTestId('aiws-cell-task-42-budget')).toContainText('1.00')
  await screenshot(owner.page, info, 'pagination-source-persisted')
  await info.attach('logic-pagination-verdict', {
    body: Buffer.from(JSON.stringify({ first_page: first.rows.map((row) => row.record_id), original_revision: first.content_rev, rejection: expired, restarted_revision: pages[0].content_rev, page_sizes: pages.map((page) => page.rows.length), records: ids, query_head_unchanged: events.head_seq, content_root: changedRoot })),
    contentType: 'application/json',
  })
})

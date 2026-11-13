/* Finding, reading and querying on a real Zone (notepads/aiws-core-api-unification-todo.md §5, §7): what an Agent does
 * with the buckyos CLI alone — it only knows "未完成" — compared with what the Desktop shows: finds the table view by
 * name, reads its SQL from `stat`, runs it (the rows the Desktop renders, in the same order), finds a select by its label
 * with `grep`, aggregates with SQL, writes with the verbs (field names and option labels) and reads back; the object
 * entry answers the same; the old table query is refused; and Workspaces written before names existed have them. */

import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { Locator } from '@playwright/test'
import type { WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { cli, createSample, expect, openWorkspace, screenshot, test } from './aiworkspace.fixtures'

type Json = Record<string, any> // eslint-disable-line @typescript-eslint/no-explicit-any

test.use({ viewport: { width: 1600, height: 1000 } })

/** Every short type: `doc.find` leaves annotations, viewports, paths and session entities out unless named. */
const ALL_TYPES = ['table', 'richtext', 'record', 'annotation', 'block', 'container', 'asset', 'dir', 'wish', 'def', 'viewport', 'path', 'task', 'response', 'result']

async function renderedIds(table: Locator) {
  return table.getByTestId('aiws-row').evaluateAll((rows) => rows.map((row) => row.getAttribute('data-record-id')))
}

test('[api] an Agent finds a table view by name, runs its SQL, writes with the verbs and reads back; the Desktop shows the same rows', async ({ owner, baseURL }, info) => {
  test.setTimeout(360_000)
  const ws = await createSample(owner)
  const token = (await owner.page.context().cookies()).find((c) => c.name === 'buckyos_session_token')!.value
  const work = mkdtempSync(join(tmpdir(), 'aiws-api-'))
  const aiws = (...args: string[]) => cli(token, baseURL!, work, 'aiws', ...args, '--workspace', ws.workspace_id) as Json

  // find → ls → stat: the view, by its name, with its SQL
  const found = aiws('find', '未完成')
  const view = (found.items as Json[]).find((i) => i.entity_id === 'cell-open-tasks')
  expect(view?.path, JSON.stringify(found)).toBe('/surfaces/项目工作区/未完成任务')
  const canvas = aiws('ls', '/surfaces/项目工作区')
  expect((canvas.items as Json[]).map((i) => i.name)).toEqual(expect.arrayContaining(['全部任务', '未完成任务', '项目信息']))
  expect((canvas.items as Json[]).every((i) => typeof i.name === 'string' && i.name.length > 0), JSON.stringify(canvas)).toBe(true)
  const meta = aiws('stat', '/surfaces/项目工作区/未完成任务')
  expect((meta.shows as Json).path).toBe('/data/任务')
  const viewSql = String((meta.shows as Json).view_sql)
  expect(viewSql).toContain('FROM "/data/任务"')

  // the view's SQL gives the rows the Desktop renders, in its order (the Desktop may evaluate them on its replica)
  const viaSql = (aiws('sql', viewSql).rows as Json[][]).map((r) => r[0])
  expect(viaSql).toEqual(['task-41', 'task-42', 'task-44', 'task-43'])
  await openWorkspace(owner, ws)
  // the canvas Block (the notes embed the same view: `aiws-embed-cell-open-tasks`)
  const table = owner.page.getByTestId('aiws-cell-frame-cell-open-tasks').getByTestId('aiws-table-cell-open-tasks')
  await expect.poll(() => renderedIds(table)).toEqual(viaSql)
  await screenshot(owner.page, info, 'view-as-desktop-shows-it')

  // a select is found by its label; SQL reads display values and stored ones
  const hits = aiws('grep', '进行中')
  const cell = (hits.hits as Json[]).find((h) => h.entity_id === 'tasks')
  expect(cell?.selector, JSON.stringify(hits)).toMatchObject({ kind: 'table_cell', field_id: 'status' })
  expect((hits.coverage as Json).complete).toBe(true)
  const totals = aiws('sql', 'SELECT "状态", count(*), sum("预算") FROM "/data/任务" GROUP BY 1 ORDER BY 1')
  expect(totals.rows).toEqual([['完成', 1, 300], ['进行中', 4, 2009.5]])
  expect((totals.read as Json[])[0]).toMatchObject({ entity_id: 'tasks', path: '/data/任务' })

  // write with field names and option labels, read back; the Desktop follows
  aiws('table', 'set', '/data/任务', 'task-43', '--values', JSON.stringify({ 状态: '完成', 预算: '50' }))
  expect(aiws('sql', `SELECT "状态", "预算#raw", "状态#raw" FROM "/data/任务" WHERE _id = 'task-43'`).rows).toEqual([['完成', '50.00', 'option-done']])
  await expect.poll(() => renderedIds(table)).toEqual(['task-41', 'task-42', 'task-44'])
  aiws('text', 'append', '/data/项目说明', '--text', 'DV 由命令行追加的一段')
  const body = aiws('cat', '/data/项目说明')
  expect(String(body.text)).toContain('DV 由命令行追加的一段')
  const csv = aiws('cat', '/data/任务')
  // the sample's records have bodies, so the header and every row end with `_body` (the body's path, empty without one)
  expect(String(csv.text).split('\n')[0]).toBe('_id,任务,状态,预算,截止日期,负责人,风险,_body')
  expect(String(csv.text)).toContain('task-43,写验收用例,完成,50.00')

  // the object entry: meta, body, the index's grep; a fixed version that is not kept is refused, not read as the head
  // from the test process with the owner's cookies (a refused fetch in the page would log a console error)
  const http = owner.page.request
  const entry = `${baseURL}/kapi/aiworkspace/obj/${ws.workspace_id}`
  const get = async (path: string) => { const r = await http.get(`${entry}${path}`, { ignoreHTTPSErrors: true }); return { status: r.status(), body: await r.json().catch(() => null) as Json } }
  const obj = {
    meta: await get('/entities/tasks/props/meta'),
    body: await get('/entities/notes/props/body'),
    fixed: await get(`/entities/tasks/props/content?mode=fixed&version=${encodeURIComponent(JSON.stringify({ epoch: 'ep_nope', seq: 1 }))}`),
    query: await (await http.post(`${entry}/methods/query?op=grep&text=${encodeURIComponent('离线')}`, { ignoreHTTPSErrors: true, data: { method: 'query', params: {} } })).json() as Json,
  }
  expect(obj.meta.status).toBe(200)
  expect(obj.meta.body).toMatchObject({ path: '/data/任务', type: 'table', body: { format: 'text/csv' } })
  expect(obj.body.body.text).toContain('DV 由命令行追加的一段')
  expect(obj.fixed.status, JSON.stringify(obj.fixed.body)).toBe(404)
  expect(JSON.stringify(obj.query)).toContain('task-44')

  // the old table query is gone
  const old = await owner.client.call<Json>('doc.query', { workspace_id: ws.workspace_id, source_id: 'tasks' })
  expect(old).toMatchObject({ ok: false, error: { code: 'INVALID_OPERATION' } })
  await info.attach('coreapi-verdict', { body: Buffer.from(JSON.stringify({ workspace: ws.workspace_id, view_sql: viewSql, rows: viaSql, totals: totals.rows })), contentType: 'application/json' })
})

test('[api] every entity of every Workspace the owner sees has a name, those written before names existed included', async ({ owner }) => {
  test.setTimeout(240_000)
  // the private schedule is created once per user and kept: on a Zone of an earlier build it predates names
  await owner.client.ok('ws.ensure', { name: 'schedule' })
  const { workspaces } = await owner.client.ok<{ workspaces: WorkspaceSummary[] }>('ws.list', {})
  const unnamed: string[] = []
  for (const ws of workspaces.slice(0, 40)) {
    let cursor: string | undefined
    for (let page = 0; page < 20; page += 1) {
      const found = await owner.client.ok<{ items: Json[]; next_cursor?: string; complete: boolean }>('doc.find', { workspace_id: ws.workspace_id, type: ALL_TYPES, limit: 200, ...(cursor ? { cursor } : {}) })
      for (const item of found.items) if (!item.name) unnamed.push(`${ws.workspace_id}/${item.entity_id}`)
      if (!found.next_cursor) break
      cursor = found.next_cursor
    }
  }
  expect(unnamed).toEqual([])
})

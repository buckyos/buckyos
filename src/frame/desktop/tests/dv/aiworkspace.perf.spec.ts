/* The core API's performance on a real Zone (notepads/aiws-core-api-unification-todo.md T0-6, T5-1): the fixture of
 * the store's acceptance (store/tests/perf_core_api.rs — 100 000 records × 20 fields and a 1000-row table) committed
 * through the gateway, then the calls an Agent and a Desktop make, timed from the browser: SQL (a count, a filtered
 * page, an aggregate, a join with a materialized side), the render of a saved view without the replica, the table's
 * CSV page by page, and a text query followed to the end. The pass conditions are the store's plus the round trip
 * (aiws-core-api-design.md §12.2). */

import type { WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { expect, test } from './aiworkspace.fixtures'

type Json = Record<string, any> // eslint-disable-line @typescript-eslint/no-explicit-any

const ROWS = 100_000
const FIELDS = 20
/** One call over the gateway: the store's condition plus the round trip. */
const SQL_CALL_MS = 1000
const VIEW_RENDER_MS = 2000
const BODY_PAGE_MS = 200
const TEXT_CALL_MS = 2000

class Rng {
  constructor(private s: bigint) {}
  next(): bigint {
    // xorshift64* with the store fixture's seed
    const m = (1n << 64n) - 1n
    this.s ^= this.s >> 12n
    this.s = (this.s ^ (this.s << 25n)) & m
    this.s ^= this.s >> 27n
    return (this.s * 0x2545F4914F6CDD1Dn) & m
  }
  below(n: number): number { return Number(this.next() % BigInt(n)) }
}

function fields() {
  const out: Json[] = [{ field_id: 'title', name: '标题', type: 'text', required: true }]
  for (let i = 1; i < FIELDS; i += 1) {
    out.push([
      { field_id: `f${i}`, name: `数值${i}`, type: 'number' },
      { field_id: `f${i}`, name: `金额${i}`, type: 'decimal', scale: 2 },
      { field_id: `f${i}`, name: `日期${i}`, type: 'date' },
      { field_id: `f${i}`, name: `状态${i}`, type: 'select', options: [{ option_id: 'a', label: '甲' }, { option_id: 'b', label: '乙' }, { option_id: 'c', label: '丙' }] },
      { field_id: `f${i}`, name: `文本${i}`, type: 'text' },
    ][i % 5])
  }
  return out
}

const pad = (n: number, w: number) => String(n).padStart(w, '0')

function records(rng: Rng, from: number, count: number) {
  return Array.from({ length: count }, (_, k) => {
    const n = from + k
    const values: Json = { title: n === ROWS - 1 ? `第 ${n} 条记录 末尾唯一词` : `第 ${n} 条记录` }
    for (let i = 1; i < FIELDS; i += 1) {
      values[`f${i}`] = [
        () => rng.below(100_000),
        () => `${rng.below(100_000)}.${pad(rng.below(100), 2)}`,
        () => `2026-${pad(1 + rng.below(12), 2)}-${pad(1 + rng.below(28), 2)}`,
        () => ['a', 'b', 'c'][rng.below(3)],
        () => `文本内容 ${rng.below(1_000_000)}`,
      ][i % 5]()
    }
    return { record_id: `r${pad(n, 6)}`, values }
  })
}

async function timed<T>(f: () => Promise<T>): Promise<[number, T]> {
  const t = performance.now()
  const v = await f()
  return [performance.now() - t, v]
}

test('[perf] 100 000 records on a Zone: SQL, a saved view without the replica, the CSV page by page and a text query, within the pass conditions', async ({ owner }, info) => {
  test.setTimeout(900_000)
  const created = await owner.client.ok<WorkspaceSummary>('ws.create', { title: owner.client.prefix })
  owner.client.owned.add(created.workspace_id)
  const ws = { workspace_id: created.workspace_id, epoch: created.epoch }
  const wsId = ws.workspace_id
  const report: Record<string, unknown> = {}
  const failures: string[] = []
  const atMost = (what: string, ms: number, max: number) => {
    report[what] = Math.round(ms * 10) / 10
    if (ms > max) failures.push(`${what}: ${ms.toFixed(1)} ms > ${max} ms`)
  }

  // the fixture through the gateway
  const loadStart = performance.now()
  const first = await owner.client.commit(ws, [
    { op: 'entity.create', entity_id: 'big', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'p', name: '大表', payload: { title_field_id: 'title', fields: fields() } },
    { op: 'entity.create', entity_id: 'dept', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'q', name: '部门',
      payload: { fields: [{ field_id: 'code', name: '编号', type: 'number' }, { field_id: 'dname', name: '名称', type: 'text' }] } },
    { op: 'entity.create', entity_id: 'perf-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'p',
      payload: { kind: 'folder', system: 'surface_content', surface_id: 'perf' } },
    { op: 'entity.create', entity_id: 'perf', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'p',
      payload: { kind: 'surface', layout: { mode: 'free' }, content_folder_id: 'perf-content', title: '性能' } },
    { op: 'entity.create', entity_id: 'view-open', type_id: 'buckyos.cell', parent_id: 'perf', order_key: 'a', placement: { x: 0, y: 0, w: 800, h: 600 },
      payload: { title: '乙类大额', source_ref: { entity_id: 'big' }, view: { type: 'table' },
                 filter: { op: 'and', args: [{ op: 'cmp', field_id: 'f3', operator: 'eq', value: 'b' }, { op: 'cmp', field_id: 'f5', operator: 'gt', value: 50000 }] },
                 sorts: [{ field_id: 'f1', direction: 'desc' }], fields: [{ field_id: 'title' }, { field_id: 'f1' }, { field_id: 'f3' }, { field_id: 'f5' }] } },
  ] as never)
  expect(first.status, JSON.stringify(first)).toBe('accepted')
  const rng = new Rng(0x5eed_1234n)
  for (let chunk = 0; chunk < ROWS / 1000; chunk += 1) {
    const r = await owner.client.commit(ws, [{ op: 'table.insert_records', source_id: 'big', records: records(rng, chunk * 1000, 1000) }] as never)
    expect(r.status, JSON.stringify(r).slice(0, 400)).toBe('accepted')
  }
  const depts = Array.from({ length: 1000 }, (_, n) => ({ record_id: `d${pad(n, 4)}`, values: { code: n, dname: `部门 ${n}` } }))
  expect((await owner.client.commit(ws, [{ op: 'table.insert_records', source_id: 'dept', records: depts }] as never)).status).toBe('accepted')
  report.load_s = Math.round((performance.now() - loadStart) / 100) / 10

  // SQL over kRPC
  const sql = (q: string, extra: Json = {}) => owner.client.ok<Json>('doc.query', { workspace_id: wsId, sql: q, ...extra })
  const queries: [string, string][] = [
    ['sql count', 'SELECT count(*) FROM "/data/大表"'],
    ['sql filter + sort page', `SELECT _id, "标题", "金额1" FROM "/data/大表" WHERE "状态3#raw" = 'b' AND "数值5" > 50000 ORDER BY ("金额1" IS NULL), "金额1" DESC, _id LIMIT 100`],
    ['sql group + sum', 'SELECT "状态3", count(*), sum("金额1") FROM "/data/大表" GROUP BY 1 ORDER BY 1'],
    ['sql join (materialized side)', 'WITH d AS MATERIALIZED (SELECT "编号" AS code, "名称" AS name FROM "/data/部门") SELECT d.name, count(*) FROM "/data/大表" b JOIN d ON (b."数值5" % 1000) = d.code GROUP BY 1 ORDER BY 2 DESC, 1 LIMIT 10'],
  ]
  for (const [what, q] of queries) {
    await sql(q)
    const [t, out] = await timed(() => sql(q))
    expect(out.rows.length).toBeGreaterThan(0)
    atMost(what, t, SQL_CALL_MS)
  }
  expect((await sql('SELECT count(*) FROM "/data/大表"')).rows[0][0]).toBe(ROWS)
  const plain = await owner.client.call<Json>('doc.query', { workspace_id: wsId, sql: 'SELECT d."名称", count(*) FROM "/data/大表" b JOIN "/data/部门" d ON (b."数值5" % 1000) = d."编号" GROUP BY 1' })
  expect(plain, 'a plain join of 100 000 × 1000 is refused with the way to write it').toMatchObject({ ok: false, error: { code: 'LIMIT_EXCEEDED' } })
  expect(JSON.stringify(plain)).toContain('AS MATERIALIZED')

  // a saved view rendered by a Desktop without the replica (tableQuery.ts: the WASM compiler + doc.query)
  await owner.client.table(ws, { view_id: 'view-open', limit: 100 })
  const [view, page] = await timed(() => owner.client.table(ws, { view_id: 'view-open', limit: 100 }))
  expect(page.rows).toHaveLength(100)
  expect(Object.keys(page.rows[0].values).sort()).toEqual(['f1', 'f3', 'f5', 'title'])
  atMost('view render without the replica', view, VIEW_RENDER_MS)
  // its parts: the reads of the view and the table, then its page and its count
  const [readMs, reads] = await timed(() => owner.client.ok<Json>('doc.read', { workspace_id: wsId, targets: [{ entity_id: 'view-open' }, { entity_id: 'big' }] }))
  report.view_reads_ms = Math.round(readMs)
  report.view_reads_kib = Math.round(JSON.stringify(reads).length / 1024)
  report.view_total = page.total

  // the table's CSV, 256 KiB pages
  const pages: number[] = []
  let cursor: string | undefined
  let lines = 0
  const csvStart = performance.now()
  for (;;) {
    const [t, out] = await timed(() => owner.client.ok<Json>('doc.read', { workspace_id: wsId, entity_id: 'big', as: 'body', max_bytes: 262144, ...(cursor ? { cursor } : {}) }))
    pages.push(t)
    lines += String(out.text).split('\n').filter((l) => l.length > 0).length
    if (out.complete || !out.next_cursor) break
    cursor = out.next_cursor
  }
  expect(lines).toBe(ROWS + 1)
  const sorted = [...pages].sort((a, b) => a - b)
  atMost('body page (256 KiB) p95', sorted[Math.floor(sorted.length * 0.95)], BODY_PAGE_MS)
  report.body_pages = pages.length
  report.body_all_ms = Math.round(performance.now() - csvStart)

  // a text query followed to the end: the hit only in the last record
  const calls: number[] = []
  const hits: Json[] = []
  cursor = undefined
  for (;;) {
    const [t, out] = await timed(() => owner.client.ok<Json>('doc.query', { workspace_id: wsId, text: '末尾唯一词', at: '/data/大表', ...(cursor ? { cursor } : {}) }))
    calls.push(t)
    hits.push(...out.hits)
    if (!out.next_cursor) { expect(out.coverage.complete).toBe(true); break }
    expect(out.coverage.complete).toBe(false)
    cursor = out.next_cursor
  }
  expect(hits.map((h) => h.selector?.record_id)).toEqual([`r${pad(ROWS - 1, 6)}`])
  atMost('text query, slowest call', Math.max(...calls), TEXT_CALL_MS)
  report.text_calls = calls.length

  await info.attach('perf-verdict', { body: Buffer.from(JSON.stringify(report, null, 2)), contentType: 'application/json' })
  console.log(JSON.stringify(report))
  expect(failures).toEqual([])
})

/* The rows of a table view for a session without a replica (核心 API 调整 §3.5): the service answers only text and SQL
 * queries, so the view's saved definition (filter, sorts, group, manual order — plus what the caller adds) is compiled
 * to SQL here by the core's one compiler (WASM `view_sql`, the same the service and the stages use) and run with
 * `doc.query { sql }`. A tab holding the replica evaluates the same definition locally with the core's evaluator; the
 * two give the same rows in the same order (aiworkspace store tests/sql.rs). URL tables keep their remote query
 * (`doc.source_query`). Paging is by offset (`best_effort`, as the scrolling views always read).
 *
 * No imports but types: the session passes its client and the WASM compiler, tests their own. */

import type { FilterNode, Json, QueryPage, QueryParams, QueryRow, SortSpec, SqlParams, SqlResult } from './types'

export interface ServiceError { code: string; detail?: string; sub_code?: string; data?: Record<string, unknown> }

export interface TableQueryDeps {
  /** `doc.read` of several entities: each result or `{ error }`. */
  read(entityIds: string[]): Promise<({ error?: ServiceError } & Record<string, unknown>)[]>
  sql(params: SqlParams): Promise<SqlResult>
  sourceQuery(params: QueryParams): Promise<QueryPage>
  /** The core's `view_sql` (WASM). */
  viewSql(table: string, fieldsJson: string, viewJson: string, projection: 'records' | 'display'): string
  /** The error to throw for a refusal of this module or of the compiler. */
  fail(error: ServiceError): Error
}

interface TableRead { content_rev: number; type_id: string; content: { payload: Record<string, Json>; fields: Json[] } }
interface CellRead { type_id: string; content: { payload: Record<string, Json> } }
interface Compiled { sql: string; count_sql: string; group_sql?: string | null; diagnostics: QueryPage['diagnostics'] }

function compile(deps: TableQueryDeps, run: () => string): Compiled {
  try {
    return JSON.parse(run()) as Compiled
  } catch (error) {
    // the compiler throws the JSON of a structured error (`VIEW_BROKEN`, …)
    const message = error instanceof Error ? error.message : String(error)
    let parsed: ServiceError | null = null
    try { parsed = JSON.parse(message) as ServiceError } catch { parsed = null }
    if (parsed && typeof parsed.code === 'string') throw deps.fail(parsed)
    throw error
  }
}

function and(a: FilterNode | null | undefined, b: FilterNode | null | undefined): FilterNode | null {
  if (a && b) return { op: 'and', args: [a, b] } as unknown as FilterNode
  return a ?? b ?? null
}

const ALIAS = 't'

export async function queryBySql(deps: TableQueryDeps, params: QueryParams): Promise<QueryPage> {
  const ids = [...(params.view_id ? [params.view_id] : []), ...(params.source_id ? [params.source_id] : [])]
  const reads = ids.length ? await deps.read(ids) : []
  for (const r of reads) if (r.error) throw deps.fail(r.error)
  const cell = params.view_id ? (reads[0] as unknown as CellRead) : null
  let table = params.source_id ? (reads[params.view_id ? 1 : 0] as unknown as TableRead) : null
  let sourceId = params.source_id
  if (cell) {
    if (cell.type_id !== 'buckyos.cell' || (cell.content.payload.view as { type?: string } | undefined)?.type !== 'table') {
      throw deps.fail({ code: 'INVALID_OPERATION', detail: 'view_id must name a live table view' })
    }
    const shown = (cell.content.payload.source_ref as { entity_id?: string } | undefined)?.entity_id
    if (shown && shown !== sourceId) {
      sourceId = shown
      table = null
    }
  }
  if (!sourceId) throw deps.fail({ code: 'INVALID_OPERATION', detail: 'source_id or view_id required' })
  if (!table) {
    const [read] = await deps.read([sourceId])
    if (read.error) throw deps.fail(read.error)
    table = read as unknown as TableRead
  }
  if (table.type_id !== 'buckyos.table-source') throw deps.fail({ code: 'INVALID_OPERATION', detail: `${sourceId} is not a table source` })
  const saved = cell?.content.payload ?? {}
  const filter = and((saved.filter ?? null) as FilterNode | null, params.filter ?? null)
  const sorts = (params.sorts ?? (saved.sorts as unknown as SortSpec[] | undefined) ?? []) as SortSpec[]
  const group = params.group ?? ((saved.group as { field_id?: string } | undefined)?.field_id ?? undefined)
  if (table.content.payload.data_mode === 'url_query') {
    const page = await deps.sourceQuery({ source_id: sourceId, ...(filter ? { filter } : {}), ...(sorts.length ? { sorts } : {}), ...(params.fields ? { fields: params.fields } : {}),
      limit: params.limit ?? 100, ...(params.cursor ? { cursor: params.cursor } : {}), ...(params.consistency ? { consistency: params.consistency } : {}) })
    return { ...page, total: page.total ?? page.rows.length, source_id: sourceId }
  }
  // a manual order counts only without sorts (the compiler and the evaluator alike)
  const view = { ...(filter ? { filter } : {}), sorts, ...(group ? { group } : {}), ...(saved.manual_order ? { manual_order: saved.manual_order } : {}) }
  const fields = table.content.fields
  const c = compile(deps, () => deps.viewSql(ALIAS, JSON.stringify(fields), JSON.stringify(view), 'records'))
  const limit = Math.max(1, Math.min(params.limit ?? 100, 1000))
  const offset = params.cursor ? Number.parseInt(params.cursor, 10) || 0 : 0
  const tables = { [ALIAS]: sourceId }
  const [page, count, groups] = await Promise.all([
    deps.sql({ sql: `${c.sql} LIMIT ? OFFSET ?`, params: [limit, offset], tables, max_rows: limit }),
    deps.sql({ sql: c.count_sql, tables }),
    c.group_sql ? deps.sql({ sql: c.group_sql, tables, max_rows: 5000 }) : Promise.resolve(null),
  ])
  const rows = page.rows.map((r) => JSON.parse(String(r[0])) as QueryRow)
  const total = Number(count.rows[0]?.[0] ?? 0)
  const out: QueryPage = {
    rows,
    total,
    next_cursor: offset + rows.length < total ? String(offset + rows.length) : null,
    consistency: 'best_effort',
    content_rev: table.content_rev,
    source_id: sourceId,
    ...(c.diagnostics && c.diagnostics.length ? { diagnostics: c.diagnostics } : {}),
  }
  if (groups) {
    let list = groups.rows.map((r) => ({ key: String(r[0] ?? ''), count: Number(r[1] ?? 0) }))
    if (params.group_order === 'count_desc') list = [...list].sort((a, b) => b.count - a.count || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0))
    if (params.group_limit) list = list.slice(0, params.group_limit)
    out.groups = list
  }
  return out
}

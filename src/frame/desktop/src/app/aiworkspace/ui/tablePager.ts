/* Rows of one table view, loaded by keyset pages through `doc.query` (best_effort consistency for
 * scrolling, design §2.5.4) and kept fresh from the change stream. Rows are loaded in order: page N
 * needs the cursor of page N-1. Only loading is done here; the DOM side is virtualised by the view. */

import { ServiceFailure } from '../api/client'
import { describeError, type WorkspaceSession } from '../api/session'
import type { CommitEvent, Diagnostic, FilterNode, Json, QueryParams, QueryRow, SortSpec } from '../api/types'
import { Emitter } from '../state/emitter'

const PAGE = 200
const MAX_PAGE = 1000

export interface PagerQuery { viewId?: string; sourceId: string; filter: FilterNode | null; sorts: SortSpec[] | null; orderDependsOnData: boolean }

export interface PagerSnapshot {
  rows: readonly QueryRow[]
  total: number
  done: boolean
  loading: boolean
  /** A query that failed (e.g. VIEW_BROKEN): shown instead of rows, never as an empty table. */
  error: { code: string; subCode?: string; text: string; data?: Record<string, unknown> } | null
  diagnostics: readonly Diagnostic[]
}

export class TablePager {
  private rows: QueryRow[] = []
  private total = 0
  private cursor: string | null = null
  private done = false
  private loading = false
  private error: PagerSnapshot['error'] = null
  private diagnostics: Diagnostic[] = []
  private generation = 0
  private wanted = 0
  private stale = false
  private disposed = true
  private everStarted = false
  private listeners = 0
  private cached: PagerSnapshot
  private readonly emitter = new Emitter()
  private readonly session: WorkspaceSession
  private readonly query: PagerQuery
  private off: (() => void) | null = null

  constructor(session: WorkspaceSession, query: PagerQuery) {
    this.session = session
    this.query = query
    this.cached = this.build()
  }

  /** The pager works only while something is subscribed (it follows the change stream meanwhile). */
  readonly subscribe = (listener: () => void) => {
    const remove = this.emitter.subscribe(listener)
    this.listeners += 1
    if (this.listeners === 1) this.start()
    return () => {
      remove()
      this.listeners -= 1
      if (this.listeners === 0) this.stop()
    }
  }

  private start() {
    this.disposed = false
    this.off = this.session.subscribeChanges((event) => this.onChange(event))
    // After a pause the stream may have moved on without us: start from a fresh read.
    this.stale = this.everStarted
    this.everStarted = true
    this.wanted = Math.max(this.wanted, PAGE)
    void this.run()
  }

  private stop() {
    this.disposed = true
    this.generation += 1
    this.loading = false
    this.off?.()
    this.off = null
  }

  snapshot = (): PagerSnapshot => this.cached

  private build(): PagerSnapshot {
    return { rows: this.rows, total: this.total, done: this.done, loading: this.loading, error: this.error, diagnostics: this.diagnostics }
  }

  private publish() {
    this.cached = this.build()
    this.emitter.emit()
  }

  private params(limit: number, cursor: string | null): QueryParams {
    return {
      ...(this.query.viewId ? { view_id: this.query.viewId } : { source_id: this.query.sourceId }),
      ...(this.query.filter ? { filter: this.query.filter } : {}),
      ...(this.query.sorts ? { sorts: this.query.sorts } : {}),
      limit,
      ...(cursor ? { cursor } : {}),
      consistency: 'best_effort',
    }
  }

  /** Make sure the row at `index` is loaded (or the end is reached). */
  ensure(index: number) {
    this.wanted = Math.max(this.wanted, index + 1)
    if (!this.loading && !this.disposed) void this.run()
  }

  private async run() {
    const generation = this.generation
    this.loading = true
    this.publish()
    try {
      while (!this.disposed && generation === this.generation) {
        if (this.stale) {
          this.stale = false
          await this.reload(generation)
          continue
        }
        if (this.done || this.rows.length >= this.wanted) break
        const limit = Math.min(MAX_PAGE, Math.max(PAGE, this.wanted - this.rows.length))
        const page = await this.session.query(this.params(limit, this.cursor))
        if (generation !== this.generation) return
        this.rows = [...this.rows, ...page.rows]
        this.total = page.total
        this.cursor = page.next_cursor ?? null
        this.done = !page.next_cursor || page.rows.length === 0
        this.diagnostics = page.diagnostics ?? []
        this.error = null
        this.publish()
      }
    } catch (error) {
      if (generation !== this.generation) return
      this.error = error instanceof ServiceFailure
        ? { code: error.code, subCode: error.subCode, text: error.message, data: error.data }
        : { code: 'UNAVAILABLE', text: describeError(error) }
    } finally {
      if (generation === this.generation) {
        this.loading = false
        this.publish()
      }
    }
  }

  /** Load the currently shown range again into a fresh buffer, then swap (no flicker, no empty state). */
  private async reload(generation: number) {
    const target = Math.max(PAGE, Math.min(this.rows.length, this.wanted))
    const rows: QueryRow[] = []
    let cursor: string | null = null
    let done = false
    let total = 0
    let diagnostics: Diagnostic[] = []
    while (!done && rows.length < target) {
      const page = await this.session.query(this.params(Math.min(MAX_PAGE, Math.max(PAGE, target - rows.length)), cursor))
      if (generation !== this.generation) return
      rows.push(...page.rows)
      total = page.total
      cursor = page.next_cursor ?? null
      done = !page.next_cursor || page.rows.length === 0
      diagnostics = page.diagnostics ?? []
    }
    this.rows = rows
    this.total = total
    this.cursor = cursor
    this.done = done
    this.diagnostics = diagnostics
    this.error = null
    this.publish()
  }

  refresh() {
    this.stale = true
    if (!this.loading && !this.disposed) void this.run()
  }

  /** An accepted write of this session: reflect it at once so the next edit starts from the new rev. */
  patchLocal(recordId: string, fieldId: string, value: Json | undefined, rev: number) {
    const index = this.rows.findIndex((row) => row.record_id === recordId)
    if (index < 0) return
    const row = this.rows[index]
    const values = { ...row.values }
    if (value === undefined) delete values[fieldId]
    else values[fieldId] = value
    const next = [...this.rows]
    next[index] = { ...row, values, revs: { ...row.revs, [fieldId]: rev }, rev }
    this.rows = next
    this.publish()
  }

  private onChange(event: CommitEvent) {
    const touched = (event.touched ?? []).filter((item) => item.entity_id === this.query.sourceId)
    if (touched.length === 0) return
    const cellsOnly = touched.every((item) => item.selector?.kind === 'table_cell' && item.change === 'value')
    if (!cellsOnly || this.query.orderDependsOnData || touched.length > 200) { this.refresh(); return }
    // Plain cell edits in a view whose membership and order cannot depend on them: re-read just those records.
    const loaded = new Set(this.rows.map((row) => row.record_id))
    const ids = [...new Set(touched.map((item) => item.selector?.record_id).filter((id): id is string => typeof id === 'string' && loaded.has(id)))]
    if (ids.length === 0) return
    const generation = this.generation
    void this.session.readMany(ids.map((record_id) => ({ entity_id: this.query.sourceId, selector: { kind: 'table_record', record_id } }))).then((results) => {
      if (generation !== this.generation || this.disposed) return
      const fresh = new Map<string, QueryRow>()
      results.forEach((result, i) => {
        if ('error' in result) return
        const content = result.content as QueryRow
        fresh.set(ids[i], content)
      })
      this.rows = this.rows.map((row) => {
        const update = fresh.get(row.record_id)
        if (!update) return row
        // keep the view's field projection: only fields the row already carries or that changed
        return { ...row, rev: update.rev, revs: update.revs, values: update.values, meta: update.meta }
      })
      this.publish()
    }, () => this.refresh())
  }

}

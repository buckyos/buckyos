/* The records a data control projects (控件机制 TODO G2): every record of a bound table with its fields, kept current
 * from the change stream (a Commit touching the table re-reads the touched records, or all of them when records come
 * or go), shared by every control showing that table. A write the control just sent shows at once as a local overlay
 * (the drop is not undone while the Commit travels) and is dropped once the stream brings the result, or at once when
 * it is refused — the control then shows what the service has. */

import { useMemo, useSyncExternalStore } from 'react'
import { ServiceFailure } from '../../../api/client'
import { describeError } from '../../../api/session'
import type { CommitEvent, FieldDef, Json, QueryRow, TableSourceContent } from '../../../api/types'
import { Emitter } from '../../../state/emitter'
import { useStore } from '../../../state/hooks'
import type { WorkspaceStore } from '../../../state/store'

/** A data control does not page: it lays out all of a table's records (a person's to-dos and events). */
const MAX_ROWS = 20_000
const PAGE = 1000

export interface RecordSnapshot {
  fields: readonly FieldDef[]
  titleField: string | null
  rows: readonly QueryRow[]
  /** The table's own envelope fields the control needs. */
  schemaId: string | null
  loading: boolean
  /** Why the records cannot be shown (the table is gone, not readable …), null otherwise. */
  error: string | null
  /** Bumped at every change, for memo keys. */
  version: number
}

type Overlay = { values: Record<string, Json | undefined>; at: number }

export class RecordSet {
  private fields: FieldDef[] = []
  private titleField: string | null = null
  private rows = new Map<string, QueryRow>()
  private order: string[] = []
  private overlays = new Map<string, Overlay>()
  private loading = true
  private error: string | null = null
  private version = 0
  private cached: RecordSnapshot
  private readonly emitter = new Emitter()
  private off: (() => void) | null = null
  private listeners = 0
  private generation = 0
  private reloadTimer: number | null = null

  private readonly store: WorkspaceStore
  readonly sourceId: string

  constructor(store: WorkspaceStore, sourceId: string) {
    this.store = store
    this.sourceId = sourceId
    this.cached = this.build()
  }

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

  snapshot = (): RecordSnapshot => this.cached

  private build(): RecordSnapshot {
    const rows: QueryRow[] = []
    for (const id of this.order) {
      const row = this.rows.get(id)
      if (!row) continue
      const overlay = this.overlays.get(id)
      if (!overlay) { rows.push(row); continue }
      const values = { ...row.values }
      for (const [k, v] of Object.entries(overlay.values)) if (v === undefined) delete values[k]; else values[k] = v
      rows.push({ ...row, values })
    }
    // records added locally and not yet read back
    for (const [id, overlay] of this.overlays) {
      if (this.rows.has(id) || overlay.values.__new === undefined) continue
      const values = { ...overlay.values }
      delete values.__new
      rows.push({ record_id: id, rev: 0, revs: {}, values: values as Record<string, Json> })
    }
    const entity = this.store.outline.get(this.sourceId)
    return { fields: this.fields, titleField: this.titleField, rows, schemaId: (entity as { schema_id?: string } | undefined)?.schema_id ?? null, loading: this.loading, error: this.error, version: this.version }
  }

  private publish() {
    this.version += 1
    this.cached = this.build()
    this.emitter.emit()
  }

  private start() {
    this.off = this.store.session.subscribeChanges((event) => this.onChange(event))
    void this.reload()
  }

  private stop() {
    this.off?.()
    this.off = null
    this.generation += 1
    if (this.reloadTimer !== null) { window.clearTimeout(this.reloadTimer); this.reloadTimer = null }
  }

  /** Read the fields and every record again. */
  async reload() {
    const generation = ++this.generation
    try {
      const source = await this.store.session.read<TableSourceContent>(this.sourceId)
      const rows: QueryRow[] = []
      let cursor: string | null = null
      for (;;) {
        const page = await this.store.session.query({ source_id: this.sourceId, limit: PAGE, consistency: 'best_effort', ...(cursor ? { cursor } : {}) })
        rows.push(...page.rows)
        cursor = page.next_cursor ?? null
        if (!cursor || page.rows.length === 0 || rows.length >= MAX_ROWS) break
      }
      if (generation !== this.generation) return
      this.fields = source.content.fields
      this.titleField = source.content.payload.title_field_id ?? null
      this.rows = new Map(rows.map((r) => [r.record_id, r]))
      this.order = rows.map((r) => r.record_id)
      this.settleOverlays()
      this.error = null
    } catch (error) {
      if (generation !== this.generation) return
      this.error = error instanceof ServiceFailure ? error.message : describeError(error)
    }
    this.loading = false
    this.publish()
  }

  private scheduleReload() {
    if (this.reloadTimer !== null) return
    this.reloadTimer = window.setTimeout(() => { this.reloadTimer = null; void this.reload() }, 30)
  }

  private onChange(event: CommitEvent) {
    const touched = (event.touched ?? []).filter((item) => item.entity_id === this.sourceId)
    if (touched.length === 0) return
    const cellsOnly = touched.every((item) => item.selector?.kind === 'table_cell' && item.change === 'value')
    if (!cellsOnly || touched.length > 200) { this.scheduleReload(); return }
    const ids = [...new Set(touched.map((item) => item.selector?.record_id).filter((id): id is string => typeof id === 'string' && this.rows.has(id)))]
    if (ids.length === 0) return
    const generation = this.generation
    void this.store.session.readMany(ids.map((record_id) => ({ entity_id: this.sourceId, selector: { kind: 'table_record', record_id } }))).then((results) => {
      if (generation !== this.generation) return
      results.forEach((result, i) => {
        if ('error' in result) return
        const row = result.content as QueryRow
        this.rows.set(ids[i], { record_id: ids[i], rev: row.rev, revs: row.revs, values: row.values, meta: row.meta, body_ref: row.body_ref })
      })
      this.settleOverlays()
      this.publish()
    }, () => this.scheduleReload())
  }

  /** An overlay is dropped once the service's values agree with it (or after a while: the stream decides). */
  private settleOverlays() {
    const now = Date.now()
    for (const [id, overlay] of this.overlays) {
      const row = this.rows.get(id)
      const agrees = row ? Object.entries(overlay.values).every(([k, v]) => k === '__new' || JSON.stringify(row.values[k]) === JSON.stringify(v)) : false
      const gone = !row && overlay.values.__new === undefined
      if (agrees || gone || now - overlay.at > 15_000) this.overlays.delete(id)
    }
  }

  /** Show `values` on a record at once (undefined: unset); `create` for a record not stored yet. */
  overlay(recordId: string, values: Record<string, Json | undefined>, create = false) {
    const prev = this.overlays.get(recordId)
    this.overlays.set(recordId, { values: { ...(prev?.values ?? {}), ...values, ...(create ? { __new: true } : {}) }, at: Date.now() })
    this.publish()
  }

  /** A write was refused: show the service's values again. */
  dropOverlay(recordIds: string[]) {
    let any = false
    for (const id of recordIds) any = this.overlays.delete(id) || any
    if (any) this.publish()
  }

  /** Hide records at once (a delete in flight). */
  hide(recordIds: string[]) {
    for (const id of recordIds) this.overlays.set(id, { values: { __hidden: true }, at: Date.now() })
    this.publish()
  }
}

const sets = new WeakMap<WorkspaceStore, Map<string, RecordSet>>()

export function recordSet(store: WorkspaceStore, sourceId: string): RecordSet {
  let byId = sets.get(store)
  if (!byId) { byId = new Map(); sets.set(store, byId) }
  let set = byId.get(sourceId)
  if (!set) { set = new RecordSet(store, sourceId); byId.set(sourceId, set) }
  return set
}

const EMPTY: RecordSnapshot = { fields: [], titleField: null, rows: [], schemaId: null, loading: false, error: null, version: 0 }

/** The live records of a bound table (none without a binding). */
export function useRecords(sourceId: string | null | undefined): { set: RecordSet | null; snapshot: RecordSnapshot } {
  const store = useStore()
  const set = useMemo(() => (sourceId ? recordSet(store, sourceId) : null), [store, sourceId])
  const snapshot = useSyncExternalStore(set?.subscribe ?? noopSubscribe, set?.snapshot ?? emptySnapshot)
  const rows = useMemo(() => snapshot.rows.filter((r) => r.values.__hidden === undefined), [snapshot.rows])
  return { set, snapshot: rows === snapshot.rows ? snapshot : { ...snapshot, rows } }
}

const noopSubscribe = () => () => {}
const emptySnapshot = () => EMPTY

/** The live records of several tables at once (a calendar's calendars and its to-do table), by table id. */
export function useRecordSets(sourceIds: readonly string[]): { sets: Map<string, RecordSet>; snapshots: Map<string, RecordSnapshot>; version: number } {
  const store = useStore()
  const key = sourceIds.join('\n')
  const sets = useMemo(() => new Map(key ? key.split('\n').map((id) => [id, recordSet(store, id)]) : []), [store, key])
  const subscribe = useMemo(() => (listener: () => void) => {
    const offs = [...sets.values()].map((s) => s.subscribe(listener))
    return () => { for (const off of offs) off() }
  }, [sets])
  // the combined version: changes whenever any of the sets publishes
  const version = useSyncExternalStore(subscribe, () => [...sets.values()].reduce((v, s) => v * 31 + s.snapshot().version, 7))
  const snapshots = useMemo(() => {
    void version
    const out = new Map<string, RecordSnapshot>()
    for (const [id, set] of sets) {
      const snap = set.snapshot()
      out.set(id, { ...snap, rows: snap.rows.filter((r) => r.values.__hidden === undefined) })
    }
    return out
  }, [sets, version])
  return { sets, snapshots, version }
}

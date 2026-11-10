/* A table plan → the operations of its import (表格数据导入 TODO §3.4): the TableSource with its fields (and a table
 * Block when the import lands on a canvas), then `table.insert_records` of at most 500 records / 1 MiB each. An import
 * is one Commit (one undo step) whatever its size: the client sends a large Commit in parts (`doc.stage`), which is
 * why one operation stays well under a part.
 *
 * Record ids are one random prefix per import plus the row number in fixed-width base 36: a table without sorting
 * lists records by id, so the rows keep the order they had in the source. */

import { keysAfter, randomId } from '../import-drawio/keys.ts'
import type { Json } from './infer.ts'
import type { TablePlan } from './plan.ts'

export const RECORDS_PER_OP = 500
/** One `table.insert_records` at most (UTF-8 JSON): a part of a Commit sent in parts is 6 MiB. */
export const OP_BYTES = 1024 * 1024

export type TableOp = { op: string; [key: string]: Json | undefined }

export interface TableTarget {
  /** The folder the TableSource goes in and the last order key among its children. */
  folderId: string
  afterKey: string | null
  /** The entity name (names are unique among siblings: the host picks a free one, `uniqueName`). */
  name: string
  /** A table Block showing it (a canvas import): its parent, the last order key there, its placement (free canvas). */
  block?: { parentId: string; afterKey: string | null; orderKey?: string; placement?: Record<string, number> | null; title?: string }
  newId?: (prefix: string) => string
  recordsPerOp?: number
  opBytes?: number
}

export interface TableOps { sourceId: string; cellId: string | null; operations: TableOp[] }

/** Names are 1–128 characters without `/` or control characters (`core/src/id.rs`), unique among siblings. */
export function uniqueName(base: string, taken: Set<string>): string {
  // deno-lint-ignore no-control-regex
  const clean = base.replace(/[/\u0000-\u001f\u007f]/g, ' ').replace(/\s+/g, ' ').trim().slice(0, 128) || 'Table'
  let name = clean
  for (let k = 2; taken.has(name); k++) { const tail = ` (${k})`; name = clean.slice(0, 128 - tail.length) + tail }
  return name
}

/** Ids `r<prefix><n>` sorting in row order. */
export function recordIds(count: number, newId: (prefix: string) => string = randomId): string[] {
  const prefix = newId('r').slice(0, 11)
  const width = Math.max(4, Math.max(1, count - 1).toString(36).length)
  return Array.from({ length: count }, (_, i) => prefix + i.toString(36).padStart(width, '0'))
}

export function tableOperations(plan: TablePlan, target: TableTarget): TableOps {
  const newId = target.newId ?? randomId
  const sourceId = newId('s')
  const perOp = target.recordsPerOp ?? RECORDS_PER_OP
  const head: TableOp[] = [{
    op: 'entity.create', entity_id: sourceId, type_id: 'buckyos.table-source', parent_id: target.folderId, order_key: keysAfter(target.afterKey, 1)[0], name: target.name,
    payload: { ...(plan.titleFieldId ? { title_field_id: plan.titleFieldId } : {}), fields: plan.fields as Json },
  }]
  let cellId: string | null = null
  if (target.block) {
    cellId = newId('c')
    const b = target.block
    head.push({
      op: 'entity.create', entity_id: cellId, type_id: 'buckyos.cell', parent_id: b.parentId, order_key: b.orderKey ?? keysAfter(b.afterKey, 1)[0],
      ...(b.placement ? { placement: b.placement } : {}),
      payload: { view: { type: 'table' }, source_ref: { entity_id: sourceId }, ...(b.title ? { title: b.title.slice(0, 256) } : {}) },
    })
  }
  const ids = recordIds(plan.records.length, newId)
  const budget = target.opBytes ?? OP_BYTES
  const operations: TableOp[] = [...head]
  let bytes = 0
  let batch: { record_id: string; values: Record<string, Json> }[] = []
  const flushOp = () => { if (batch.length) { operations.push({ op: 'table.insert_records', source_id: sourceId, records: batch as unknown as Json }); batch = []; bytes = 0 } }
  for (let i = 0; i < plan.records.length; i++) {
    const record = { record_id: ids[i], values: plan.records[i] }
    const size = utf8Length(JSON.stringify(record)) + 1
    if (batch.length >= perOp || (batch.length > 0 && bytes + size > budget)) flushOp()
    batch.push(record)
    bytes += size
  }
  flushOp()
  return { sourceId, cellId, operations }
}

/** UTF-8 length of a string. */
function utf8Length(s: string): number {
  let n = s.length
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i)
    if (c > 0x7f) n += c > 0x7ff && (c < 0xd800 || c > 0xdfff) ? 2 : 1
  }
  return n
}

/** The size of a table Block on a free canvas (§3.5): wide enough for its columns, high enough for a dozen rows. */
export function blockSize(columns: number, rows: number): { w: number; h: number } {
  return { w: Math.min(960, 40 + 150 * columns), h: Math.min(480, 70 + 34 * Math.min(rows, 12)) }
}
